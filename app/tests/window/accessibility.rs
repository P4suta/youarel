use gio::glib;
use std::future::Future;
use std::time::Duration;

#[derive(Debug)]
struct Control {
    name: String,
    text: Option<String>,
}

/// Waits for a real focused control, optionally with its complete text.
///
/// # Panics
///
/// Panics if the control doesn't reach the requested state before the deadline.
pub fn wait_for(name: &str, text: Option<&str>) {
    glib::MainContext::new()
        .block_on(wait_with(name, text, super::WAIT, raw::observe))
        .unwrap();
}

async fn wait_with<O, F>(
    name: &str,
    text: Option<&str>,
    timeout: Duration,
    mut observe: O,
) -> anyhow::Result<()>
where
    O: FnMut() -> F,
    F: Future<Output = anyhow::Result<Vec<Control>>>,
{
    use futures_util::future::{Either, select};

    let mut observed = None;
    let polling = async {
        loop {
            observed = Some(observe().await);
            if observed.as_ref().is_some_and(|result| {
                result.as_ref().is_ok_and(|controls| {
                    controls.iter().any(|control| {
                        control.name == name
                            && text.is_none_or(|expected| control.text.as_deref() == Some(expected))
                    })
                })
            }) {
                return;
            }
            glib::timeout_future(super::POLL).await;
        }
    };
    match select(Box::pin(polling), glib::timeout_future(timeout)).await {
        Either::Left(_) => return Ok(()),
        Either::Right(((), pending)) => drop(pending),
    }
    anyhow::bail!(
        "readiness deadline expired for {name:?} with text {text:?}; last completed observation: {observed:?}"
    )
}

mod raw {
    use super::Control;
    use gio::glib::{self, variant::ObjectPath};
    use std::collections::{HashSet, VecDeque};

    const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
    const ROOT: &str = "/org/a11y/atspi/accessible/root";
    const REGISTRY: &str = "org.a11y.atspi.Registry";
    const TEXT: &str = "org.a11y.atspi.Text";
    const FOCUSED: u32 = 1 << 12;
    const CALL_MS: i32 = 1_000;
    const MAX_OBJECTS: usize = 2_000;

    #[derive(Debug)]
    struct Desktop(gio::DBusConnection);

    #[expect(
        clippy::future_not_send,
        reason = "GIO futures stay on the GLib main context that owns each operation"
    )]
    pub(super) async fn observe() -> anyhow::Result<Vec<Control>> {
        Desktop::connect().await?.focused().await
    }

    #[expect(
        clippy::future_not_send,
        reason = "GIO futures stay on the GLib main context that owns each operation"
    )]
    impl Desktop {
        async fn connect() -> anyhow::Result<Self> {
            let session = gio::bus_get_future(gio::BusType::Session).await?;
            let (address,): (String,) = call(
                &session,
                ("org.a11y.Bus", "/org/a11y/bus"),
                ("org.a11y.Bus", "GetAddress"),
                None,
            )
            .await?;
            Ok(Self(
                gio::DBusConnection::for_address_future(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                )
                .await?,
            ))
        }

        async fn focused(&self) -> anyhow::Result<Vec<Control>> {
            let mut pending = VecDeque::from([(REGISTRY.to_owned(), ObjectPath::try_from(ROOT)?)]);
            let mut seen = HashSet::new();
            let mut controls = Vec::new();
            while let Some((bus, path)) = pending.pop_front() {
                if !seen.insert((bus.clone(), path.clone())) {
                    continue;
                }
                anyhow::ensure!(
                    seen.len() <= MAX_OBJECTS,
                    "accessibility tree exceeds its bound"
                );
                let (states,): (Vec<u32>,) =
                    call(&self.0, (&bus, &path), (ACCESSIBLE, "GetState"), None).await?;
                if states.first().is_some_and(|state| state & FOCUSED != 0) {
                    use glib::variant::ToVariant as _;
                    let (name,): (glib::Variant,) = call(
                        &self.0,
                        (&bus, &path),
                        ("org.freedesktop.DBus.Properties", "Get"),
                        Some(&(ACCESSIBLE, "Name").to_variant()),
                    )
                    .await?;
                    let (interfaces,): (Vec<String>,) =
                        call(&self.0, (&bus, &path), (ACCESSIBLE, "GetInterfaces"), None).await?;
                    let text = if interfaces.iter().any(|interface| interface == TEXT) {
                        let (text,) = call(
                            &self.0,
                            (&bus, &path),
                            (TEXT, "GetText"),
                            Some(&(0_i32, -1_i32).to_variant()),
                        )
                        .await?;
                        Some(text)
                    } else {
                        None
                    };
                    controls.push(Control {
                        name: name
                            .get()
                            .ok_or_else(|| anyhow::anyhow!("invalid accessible name"))?,
                        text,
                    });
                }
                let (children,): (Vec<(String, ObjectPath)>,) =
                    call(&self.0, (&bus, &path), (ACCESSIBLE, "GetChildren"), None).await?;
                pending.extend(children);
            }
            Ok(controls)
        }
    }

    #[expect(
        clippy::future_not_send,
        reason = "GIO futures stay on the GLib main context that owns each operation"
    )]
    async fn call<T: glib::variant::FromVariant>(
        connection: &gio::DBusConnection,
        (bus, path): (&str, &str),
        (interface, method): (&str, &str),
        parameters: Option<&glib::Variant>,
    ) -> anyhow::Result<T> {
        let reply = connection
            .call_future(
                Some(bus),
                path,
                interface,
                method,
                parameters,
                None,
                gio::DBusCallFlags::NONE,
                CALL_MS,
            )
            .await?;
        reply
            .get()
            .ok_or_else(|| anyhow::anyhow!("invalid {interface}.{method} reply: {reply:?}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Control, wait_with};
    use futures_util::future::{Either, pending, ready, select};
    use gio::glib;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    const TEST_WAIT: Duration = Duration::from_secs(2);
    const DEADLINE: Duration = Duration::from_millis(20);
    const OUTER_LIMIT: Duration = Duration::from_millis(200);
    const ADDRESS: &str = "https://example.com/";

    #[test]
    fn connection_errors_and_incomplete_input_can_become_ready() {
        let mut attempts = 0;
        glib::MainContext::new()
            .block_on(wait_with("URL", Some(ADDRESS), TEST_WAIT, || {
                attempts += 1;
                ready(match attempts {
                    1 => Err(anyhow::anyhow!("accessibility activation pending")),
                    2 => Ok(vec![Control {
                        name: "URL".to_owned(),
                        text: Some("https://".to_owned()),
                    }]),
                    _ => Ok(vec![Control {
                        name: "URL".to_owned(),
                        text: Some(ADDRESS.to_owned()),
                    }]),
                })
            }))
            .unwrap();
        assert_eq!(attempts, 3);
    }

    #[test]
    fn timeout_keeps_the_last_connection_error() {
        let error = glib::MainContext::new()
            .block_on(wait_with("URL", Some(ADDRESS), DEADLINE, || {
                ready(Err(anyhow::anyhow!("accessibility activation pending")))
            }))
            .unwrap_err();
        let diagnostic = error.to_string();
        assert!(diagnostic.contains("URL"), "{diagnostic}");
        assert!(
            diagnostic.contains("accessibility activation pending"),
            "{diagnostic}"
        );
    }

    struct Dropped(Rc<Cell<bool>>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    #[test]
    fn the_deadline_cancels_an_unfinished_observation() {
        let dropped = Rc::new(Cell::new(false));
        let result = glib::MainContext::new().block_on(select(
            Box::pin(wait_with("URL", Some(ADDRESS), DEADLINE, || {
                let dropped = Dropped(Rc::clone(&dropped));
                async move {
                    let _held = dropped;
                    pending::<anyhow::Result<Vec<Control>>>().await
                }
            })),
            glib::timeout_future(OUTER_LIMIT),
        ));
        match result {
            Either::Left((result, _)) => {
                result.unwrap_err();
            }
            Either::Right(_) => panic!("the observation outlived the total readiness deadline"),
        }
        assert!(
            dropped.get(),
            "the timed-out operation must release its pending future"
        );
    }
}
