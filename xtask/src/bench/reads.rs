use super::{Campaign, Measurement, NOW, data::Generator};
use anyhow::{Result, ensure};
use youarel_core::library_rules::{Scope, Sort};
use youarel_core::state::{ItemId, Shortcut};
use youarel_store::{Listing, Part, Parts, Span, StoreError, Within};

pub(super) const PAGE: usize = 200;
const SORTS: [Sort; 5] = [
    Sort::Title,
    Sort::Link,
    Sort::Newest,
    Sort::Oldest,
    Sort::Opened,
];

fn spans(folders: &[ItemId]) -> Vec<(&'static str, Span)> {
    let mut spans = vec![
        ("all", Span::Scope(Scope::All)),
        ("unsorted", Span::Scope(Scope::Unsorted)),
        ("recent", Span::Scope(Scope::Recent)),
        ("home", Span::Home(None)),
    ];
    if let Some(folder) = folders.first() {
        spans.push(("collection", Span::Scope(Scope::Collection(*folder))));
        spans.push(("folder_home", Span::Home(Some(*folder))));
    }
    spans
}

fn count(parts: &Parts) -> usize {
    match parts {
        Parts::Indexed(parts) => parts.iter().fold(0usize, |total, (_within, count)| {
            total.saturating_add(*count)
        }),
        Parts::Gathered(parts) => parts.iter().fold(0usize, |total, (_group, ids)| {
            total.saturating_add(ids.len())
        }),
    }
}

fn hydrate(
    session: &youarel_store::Session,
    ids: Vec<ItemId>,
) -> Result<Vec<Shortcut>, StoreError> {
    ids.into_iter()
        .map(|id| {
            session
                .item(id)?
                .ok_or(StoreError::Missing("a synthetic page row"))
        })
        .collect()
}

impl Campaign {
    pub(super) fn reads(&self, folders: &[ItemId]) -> Result<()> {
        let store = self.store("source");
        let first = Part {
            span: Span::Scope(Scope::All),
            sort: Sort::Title,
            within: Within::Whole,
        };
        self.measure(Measurement::operation("open_to_first_page", 1), |metric| {
            let rows = store.session(NOW, |session| {
                session.parts(&Listing::of(first.span, first.sort))?;
                hydrate(session, session.page(&first, 0, PAGE)?)
            })?;
            metric.rows = rows.len();
            ensure!(
                !rows.is_empty(),
                "the synthetic library has no visible page"
            );
            std::hint::black_box(rows);
            Ok(())
        })?;
        let mut random = Generator::new(self.options.workload.seed());
        for (name, span) in spans(folders) {
            let total = store.session(NOW, |session| {
                session.parts(&Listing::of(span, Sort::Title))
            })?;
            let random_offset =
                random.pick(count(&total.parts).saturating_sub(PAGE).saturating_add(1))?;
            for sort in SORTS {
                let part = Part {
                    span,
                    sort,
                    within: Within::Whole,
                };
                for (label, offset) in [("page_first", 0), ("page_random", random_offset)] {
                    let mut measurement = Measurement::operation(label, 1);
                    measurement.scope = Some(name.to_owned());
                    measurement.sort = Some(format!("{sort:?}"));
                    measurement.offset = offset;
                    self.measure(measurement, |metric| {
                        let rows = store.session(NOW, |session| {
                            hydrate(session, session.page(&part, offset, PAGE)?)
                        })?;
                        metric.rows = rows.len();
                        std::hint::black_box(rows);
                        Ok(())
                    })?;
                }
            }
            for query in ["r", "re", "design"] {
                let mut listing = Listing::of(span, Sort::Title);
                query.clone_into(&mut listing.search);
                let mut measurement =
                    Measurement::operation(&format!("search_{}", query.chars().count()), 1);
                measurement.scope = Some(name.to_owned());
                self.measure(measurement, |metric| {
                    let grouped = store.session(NOW, |session| session.parts(&listing))?;
                    metric.rows = count(&grouped.parts);
                    std::hint::black_box(grouped);
                    Ok(())
                })?;
            }
        }
        Ok(())
    }
}
