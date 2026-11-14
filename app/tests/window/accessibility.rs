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

#[derive(Debug)]
struct Control {
    name: String,
    text: Option<String>,
}

/// Waits for a real focused control, optionally with its complete text.
///
/// # Panics
///
/// Panics if the accessibility bus fails or the control never reaches the requested state.
pub fn wait_for(name: &str, text: Option<&str>) {
    Desktop::connect().unwrap().wait_for(name, text);
}

impl Desktop {
    fn wait_for(&self, name: &str, text: Option<&str>) {
        let mut observed = Ok(Vec::new());
        assert!(
            super::eventually(|| {
                observed = self.focused();
                observed.as_ref().is_ok_and(|controls| {
                    controls.iter().any(|control| {
                        control.name == name
                            && text.is_none_or(|expected| control.text.as_deref() == Some(expected))
                    })
                })
            }),
            "expected focused control {name:?} with text {text:?}; observed {observed:?}"
        );
    }

    fn connect() -> anyhow::Result<Self> {
        let session = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE)?;
        let (address,): (String,) = call(
            &session,
            ("org.a11y.Bus", "/org/a11y/bus"),
            ("org.a11y.Bus", "GetAddress"),
            None,
        )?;
        Ok(Self(gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )?))
    }

    fn focused(&self) -> anyhow::Result<Vec<Control>> {
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
                call(&self.0, (&bus, &path), (ACCESSIBLE, "GetState"), None)?;
            if states.first().is_some_and(|state| state & FOCUSED != 0) {
                use glib::variant::ToVariant as _;
                let (name,): (glib::Variant,) = call(
                    &self.0,
                    (&bus, &path),
                    ("org.freedesktop.DBus.Properties", "Get"),
                    Some(&(ACCESSIBLE, "Name").to_variant()),
                )?;
                let (interfaces,): (Vec<String>,) =
                    call(&self.0, (&bus, &path), (ACCESSIBLE, "GetInterfaces"), None)?;
                let text = if interfaces.iter().any(|interface| interface == TEXT) {
                    let (text,) = call(
                        &self.0,
                        (&bus, &path),
                        (TEXT, "GetText"),
                        Some(&(0_i32, -1_i32).to_variant()),
                    )?;
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
                call(&self.0, (&bus, &path), (ACCESSIBLE, "GetChildren"), None)?;
            pending.extend(children);
        }
        Ok(controls)
    }
}

fn call<T: glib::variant::FromVariant>(
    connection: &gio::DBusConnection,
    (bus, path): (&str, &str),
    (interface, method): (&str, &str),
    parameters: Option<&glib::Variant>,
) -> anyhow::Result<T> {
    let reply = connection.call_sync(
        Some(bus),
        path,
        interface,
        method,
        parameters,
        None,
        gio::DBusCallFlags::NONE,
        CALL_MS,
        gio::Cancellable::NONE,
    )?;
    reply
        .get()
        .ok_or_else(|| anyhow::anyhow!("invalid {interface}.{method} reply: {reply:?}"))
}
