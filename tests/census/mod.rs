//! Whole-install censuses: a pass of our reader over every file of one format in an install, checked against an
//! oracle we did not write, with what the files hold counted. The tests assert on a census's report
//! (`tests/flver_census.rs`); `examples/census.rs` prints it, keeps it, and lists what changed since the last run.
#![allow(dead_code)]

pub mod flver;

use std::collections::BTreeMap;

/// What a census found. Each archive entry's check fills one; they add up in hash order ([`Report::add`]), so the
/// report is the same on any number of threads.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// Totals by name: the files and records read, and each condition the census counts.
    pub counts: BTreeMap<String, u64>,
    /// For each field the census follows (the fields whose meaning is not known among them), how many records hold
    /// each value. A field name may carry the condition it is counted under: `header 0x5d (version 0x2001a)`.
    pub values: BTreeMap<String, BTreeMap<i64, u64>>,
    /// The largest of a measure over every record, by name.
    pub maxima: BTreeMap<String, f64>,
    /// The files both our reader and the oracle refuse, with both reasons.
    pub agreed_refusals: Vec<String>,
    /// Every disagreement with the oracle: a value that differs, a file one side refuses, a record the other side has
    /// and ours never reached. A census passes when there is none.
    pub failures: Vec<String>,
}

impl Report {
    /// Adds `n` to the count `name`.
    pub fn count(&mut self, name: &str, n: u64) {
        // the name is copied only the first time: a census counts millions of records under a few dozen names
        match self.counts.get_mut(name) {
            Some(total) => *total += n,
            None => {
                self.counts.insert(name.to_owned(), n);
            }
        }
    }

    /// Counts one under `name` when `condition` holds; the name is listed (at zero) either way, so a condition that
    /// never holds shows as 0 rather than missing.
    pub fn flag(&mut self, name: &str, condition: bool) {
        self.count(name, u64::from(condition));
    }

    /// Counts one record holding `value` in `field`.
    pub fn value(&mut self, field: &str, value: impl Into<i64>) {
        let values = match self.values.get_mut(field) {
            Some(values) => values,
            None => self.values.entry(field.to_owned()).or_default(),
        };
        *values.entry(value.into()).or_default() += 1;
    }

    /// Raises the maximum `name` to `x`.
    pub fn maximum(&mut self, name: &str, x: f64) {
        match self.maxima.get_mut(name) {
            Some(m) => *m = m.max(x),
            None => {
                self.maxima.insert(name.to_owned(), x);
            }
        }
    }

    /// The count `name`, 0 when nothing was counted under it.
    pub fn get(&self, name: &str) -> u64 {
        self.counts.get(name).copied().unwrap_or(0)
    }

    /// Adds the report of other files to this one.
    pub fn add(&mut self, other: Report) {
        for (name, n) in other.counts {
            *self.counts.entry(name).or_default() += n;
        }
        for (field, values) in other.values {
            let into = self.values.entry(field).or_default();
            for (value, n) in values {
                *into.entry(value).or_default() += n;
            }
        }
        for (name, x) in other.maxima {
            self.maximum(&name, x);
        }
        self.agreed_refusals.extend(other.agreed_refusals);
        self.failures.extend(other.failures);
    }

    /// The report as text: the counts, each field's values (most common first), the agreed refusals and the failures.
    pub fn text(&self, title: &str) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(out, "{title}");
        let _ = writeln!(out, "  counts:");
        for (name, n) in &self.counts {
            let _ = writeln!(out, "    {name}: {n}");
        }
        if !self.maxima.is_empty() {
            let _ = writeln!(out, "  maxima:");
            for (name, x) in &self.maxima {
                let _ = writeln!(out, "    {name}: {x}");
            }
        }
        let _ = writeln!(out, "  values (value x records, most records first):");
        for (field, values) in &self.values {
            let total: u64 = values.values().sum();
            let mut by_count: Vec<(&i64, &u64)> = values.iter().collect();
            by_count.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let shown: Vec<String> = by_count
                .iter()
                .take(12)
                .map(|(v, n)| format!("{} x{n}", show(**v)))
                .collect();
            let more = by_count.len().saturating_sub(12);
            let _ = writeln!(
                out,
                "    {field}: {} value(s) over {total}: {}{}",
                values.len(),
                shown.join(", "),
                if more > 0 {
                    format!(", and {more} more")
                } else {
                    String::new()
                }
            );
        }
        let _ = writeln!(out, "  agreed refusals: {}", self.agreed_refusals.len());
        for r in &self.agreed_refusals {
            let _ = writeln!(out, "    {r}");
        }
        let _ = writeln!(out, "  failures: {}", self.failures.len());
        for f in self.failures.iter().take(40) {
            let _ = writeln!(out, "    FAIL {f}");
        }
        if self.failures.len() > 40 {
            let _ = writeln!(out, "    and {} more", self.failures.len() - 40);
        }
        out
    }

    /// What differs from `before` (the last run's report of the same census): counts, values, maxima, and the
    /// refusals and failures that came or went. Empty when nothing did.
    pub fn changes(&self, before: &Report) -> Vec<String> {
        let mut out = Vec::new();
        for name in keys(&before.counts, &self.counts) {
            let (was, now) = (before.counts.get(name), self.counts.get(name));
            if was != now {
                out.push(format!("count {name}: {} -> {}", opt(was), opt(now)));
            }
        }
        let none = BTreeMap::new();
        for field in keys(&before.values, &self.values) {
            let (was, now) = (
                before.values.get(field).unwrap_or(&none),
                self.values.get(field).unwrap_or(&none),
            );
            for value in keys(was, now) {
                let (a, b) = (was.get(value), now.get(value));
                if a != b {
                    out.push(format!(
                        "{field} = {}: {} -> {} records",
                        show(*value),
                        opt(a),
                        opt(b)
                    ));
                }
            }
        }
        for name in keys(&before.maxima, &self.maxima) {
            let (was, now) = (before.maxima.get(name), self.maxima.get(name));
            if was != now {
                out.push(format!("maximum {name}: {was:?} -> {now:?}"));
            }
        }
        for (what, was, now) in [
            ("agreed refusal", &before.agreed_refusals, &self.agreed_refusals),
            ("failure", &before.failures, &self.failures),
        ] {
            for gone in was.iter().filter(|x| !now.contains(x)) {
                out.push(format!("{what} gone: {gone}"));
            }
            for new in now.iter().filter(|x| !was.contains(x)) {
                out.push(format!("{what} new: {new}"));
            }
        }
        out
    }
}

/// A value as the report prints it: decimal, with the hexadecimal beside it from 10 up.
fn show(v: i64) -> String {
    if (0..10).contains(&v) {
        v.to_string()
    } else {
        format!("{v} ({v:#x})")
    }
}

fn opt(n: Option<&u64>) -> String {
    n.map_or_else(|| "none".to_owned(), u64::to_string)
}

/// The keys of both maps, in order, each once.
fn keys<'a, K: Ord, V>(a: &'a BTreeMap<K, V>, b: &'a BTreeMap<K, V>) -> std::collections::BTreeSet<&'a K> {
    a.keys().chain(b.keys()).collect()
}
