//! Rendering: takes rows, does not fetch them. Turns store rows into the text a tool
//! returns.

use withings_core::models::Bucket;
use withings_core::store::ecg::EcgRow;
use withings_core::store::measures::{MeasureRow, TrendRow};
use withings_core::store::people::PersonRow;
use withings_core::store::sql::SqlResult;
use withings_core::store::sync::StatusRow;

fn type_label(code: i32, name: Option<&str>) -> String {
    match name {
        Some(n) => n.to_owned(),
        None => format!("type {code}"),
    }
}

fn trimmed(value: f64) -> String {
    // Measured values are already scaled; print without trailing noise.
    let s = format!("{value:.4}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_owned()
    } else {
        s.to_owned()
    }
}

pub fn status(loops: &[StatusRow], people: &[PersonRow]) -> String {
    let mut out = String::from("Withings archive status\n");
    if loops.is_empty() {
        out.push_str("  no sync has run yet\n");
    }
    for l in loops {
        out.push_str(&format!("  loop {}: ", l.loop_name));
        match &l.last_ok_at {
            Some(ok) => out.push_str(&format!("last ok {}", ok.to_rfc3339())),
            None => out.push_str("no successful run yet"),
        }
        if let Some(day) = l.budget_day {
            out.push_str(&format!(", budget {} spent on {}", l.budget_spent, day));
        }
        out.push_str(&format!(
            ", backfill {}",
            if l.backfill_done { "done" } else { "running" }
        ));
        if let Some(err) = &l.last_error {
            out.push_str(&format!(", last error: {err}"));
        }
        out.push('\n');
    }
    out.push_str("Profiles:\n");
    for p in people {
        let name = p.label.clone().unwrap_or_else(|| format!("user {}", p.id));
        out.push_str(&format!(
            "  [{}] {}{} — {} weigh-ins{}\n",
            p.id,
            name,
            if p.is_owner {
                " (owner)"
            } else if p.shared {
                " (shared)"
            } else {
                ""
            },
            p.weigh_ins,
            match (&p.first_at, &p.last_at) {
                (Some(f), Some(l)) => format!(", {} to {}", f.date_naive(), l.date_naive()),
                _ => String::new(),
            }
        ));
    }
    out
}

pub fn people(people: &[PersonRow]) -> String {
    if people.is_empty() {
        return "No profiles archived yet.".to_owned();
    }
    let mut out = String::from("Profiles\n");
    for p in people {
        let name = p.label.clone().unwrap_or_else(|| format!("user {}", p.id));
        out.push_str(&format!(
            "  [{}] {}{} — {} weigh-ins\n",
            p.id,
            name,
            if p.is_owner {
                " (owner)"
            } else if p.shared {
                " (shared)"
            } else {
                ""
            },
            p.weigh_ins
        ));
    }
    out
}

pub fn measurements(rows: &[MeasureRow]) -> String {
    if rows.is_empty() {
        return "No measurements for that query. `status` says how much is archived yet."
            .to_owned();
    }
    let mut out = String::new();
    let mut current: Option<i64> = None;
    for r in rows {
        if current != Some(r.grpid) {
            out.push_str(&format!(
                "\n{} [grp {}]{}\n",
                r.ts.to_rfc3339(),
                r.grpid,
                if r.deleted { " (deleted)" } else { "" }
            ));
            current = Some(r.grpid);
        }
        out.push_str(&format!(
            "  {}: {}\n",
            type_label(r.type_code, r.type_name.as_deref()),
            trimmed(r.value)
        ));
    }
    out.trim_start().to_owned()
}

pub fn trend(type_: &str, bucket: Bucket, rows: &[TrendRow]) -> String {
    if rows.is_empty() {
        return format!("No {type_} measurements in that range.");
    }
    let mut out = format!("{type_} per {}\n", bucket.trunc());
    for r in rows {
        out.push_str(&format!(
            "  {}: avg {} (min {}, max {}, n={})\n",
            r.bucket.date_naive(),
            trimmed(r.avg),
            trimmed(r.min),
            trimmed(r.max),
            r.count
        ));
    }
    out
}

pub fn ecg(rows: &[EcgRow]) -> String {
    if rows.is_empty() {
        return "No ECG recordings archived.".to_owned();
    }
    let mut out = String::from("ECG recordings\n");
    for r in rows {
        out.push_str(&format!(
            "  [{}] {}{}{}\n",
            r.id,
            r.ts.to_rfc3339(),
            r.heart_rate
                .map(|h| format!(", {h} bpm"))
                .unwrap_or_default(),
            r.classification
                .as_deref()
                .map(|c| format!(", {c}"))
                .unwrap_or_default(),
        ));
    }
    out
}

pub fn sql(result: &SqlResult) -> String {
    if result.rows.is_empty() {
        return "0 rows.".to_owned();
    }
    let mut out = String::new();
    for row in &result.rows {
        out.push_str(&serde_json::to_string(row).unwrap_or_default());
        out.push('\n');
        if out.len() > 100_000 {
            out.push_str("… output truncated at 100 KB\n");
            return out;
        }
    }
    if result.truncated {
        out.push_str("… more rows; narrow the query or add LIMIT\n");
    }
    out
}
