//! Repeating tasks and reminders: `🔁 каждый понедельник`, `🔁 1-го числа`,
//! `🔁 every week`. When a repeating task is done, its next occurrence is
//! added as a new task; a repeating reminder line moves to its next time.

use std::sync::LazyLock;

use chrono::{Datelike, Duration, Months, NaiveDate, Weekday};
use regex::Regex;

/// The rule after 🔁, up to the next task marker or tag.
pub static RECUR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"🔁\x{FE0F}?\s*([^📅⏳✅🛫➕⏰⏫🔼🔽🔺⏬#\n]+)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Days(u32),
    /// Monday to Friday.
    Workdays,
    Weekday(Weekday),
    /// Every `n` months, on `day` (the same day as before when `None`).
    Months { n: u32, day: Option<u32> },
    Years(u32),
}

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,3})").unwrap());
static MONTH_DAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{1,2})\s*(?:-?(?:го|е|ое))?\s*числ|on the (\d{1,2})(?:st|nd|rd|th)?").unwrap());

const WEEKDAYS: [(&str, &str, Weekday); 7] = [
    ("понедельн", "monday", Weekday::Mon),
    ("вторн", "tuesday", Weekday::Tue),
    ("сред", "wednesday", Weekday::Wed),
    ("четверг", "thursday", Weekday::Thu),
    ("пятниц", "friday", Weekday::Fri),
    ("суббот", "saturday", Weekday::Sat),
    ("воскресен", "sunday", Weekday::Sun),
];

/// Understands Russian («каждый день», «каждые 2 недели», «по понедельникам»,
/// «каждый будний день», «1-го числа», «каждый год») and English
/// (Obsidian Tasks style: «every week», «every month on the 1st»).
pub fn parse_rule(text: &str) -> Option<Rule> {
    let t = text.trim().to_lowercase();
    let n = NUMBER.captures(&t).and_then(|c| c[1].parse::<u32>().ok()).filter(|n| *n > 0);
    if let Some(c) = MONTH_DAY.captures(&t) {
        let day = c.get(1).or(c.get(2))?.as_str().parse::<u32>().ok().filter(|d| (1..=31).contains(d))?;
        return Some(Rule::Months { n: 1, day: Some(day) });
    }
    if t.contains("будн") || t.contains("weekday") || t.contains("рабоч") {
        return Some(Rule::Workdays);
    }
    if let Some(&(_, _, w)) = WEEKDAYS.iter().find(|(ru, en, _)| t.contains(ru) || t.contains(en)) {
        return Some(Rule::Weekday(w));
    }
    let unit = |ru: &[&str], en: &[&str]| ru.iter().any(|s| t.contains(s)) || en.iter().any(|s| t.contains(s));
    if unit(&["дн", "день", "ежедневн"], &["day", "daily"]) {
        return Some(Rule::Days(n.unwrap_or(1)));
    }
    if unit(&["недел", "еженедельн"], &["week"]) {
        return Some(Rule::Days(7 * n.unwrap_or(1)));
    }
    if unit(&["месяц", "ежемесячн"], &["month"]) {
        return Some(Rule::Months { n: n.unwrap_or(1), day: None });
    }
    if unit(&["год", "лет", "ежегодн"], &["year"]) {
        return Some(Rule::Years(n.unwrap_or(1)));
    }
    None
}

/// `day` of the month of `date`, or its last day if the month is shorter.
fn on_day(date: NaiveDate, day: u32) -> NaiveDate {
    (1..=day).rev().find_map(|d| date.with_day(d)).unwrap_or(date)
}

/// The first occurrence strictly after `base`.
pub fn next_after(rule: Rule, base: NaiveDate) -> NaiveDate {
    match rule {
        Rule::Days(n) => base + Duration::days(n as i64),
        Rule::Workdays => {
            let mut d = base + Duration::days(1);
            while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                d += Duration::days(1);
            }
            d
        }
        Rule::Weekday(w) => {
            let ahead = (w.num_days_from_monday() as i64 - base.weekday().num_days_from_monday() as i64).rem_euclid(7);
            base + Duration::days(if ahead == 0 { 7 } else { ahead })
        }
        Rule::Months { n, day } => match day {
            Some(day) => {
                let this = on_day(base, day);
                if this > base && n == 1 {
                    this
                } else {
                    on_day(base.with_day(1).unwrap() + Months::new(n), day)
                }
            }
            None => base + Months::new(n),
        },
        Rule::Years(n) => base + Months::new(12 * n),
    }
}

/// The first occurrence after `base` that is not before `from`.
pub fn next_from(rule: Rule, base: NaiveDate, from: NaiveDate) -> NaiveDate {
    let mut d = next_after(rule, base);
    while d < from {
        d = next_after(rule, d);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn rules() {
        let cases = [
            ("каждый день", Rule::Days(1)),
            ("ежедневно", Rule::Days(1)),
            ("каждые 3 дня", Rule::Days(3)),
            ("каждую неделю", Rule::Days(7)),
            ("каждые 2 недели", Rule::Days(14)),
            ("каждый понедельник", Rule::Weekday(Weekday::Mon)),
            ("по пятницам", Rule::Weekday(Weekday::Fri)),
            ("каждую среду", Rule::Weekday(Weekday::Wed)),
            ("каждое воскресенье", Rule::Weekday(Weekday::Sun)),
            ("каждый будний день", Rule::Workdays),
            ("по будням", Rule::Workdays),
            ("каждый месяц", Rule::Months { n: 1, day: None }),
            ("каждые 3 месяца", Rule::Months { n: 3, day: None }),
            ("1-го числа", Rule::Months { n: 1, day: Some(1) }),
            ("каждое 15 число", Rule::Months { n: 1, day: Some(15) }),
            ("каждый год", Rule::Years(1)),
            ("every week", Rule::Days(7)),
            ("every 2 days", Rule::Days(2)),
            ("every monday", Rule::Weekday(Weekday::Mon)),
            ("every weekday", Rule::Workdays),
            ("every month on the 1st", Rule::Months { n: 1, day: Some(1) }),
            ("every year", Rule::Years(1)),
        ];
        for (text, rule) in cases {
            assert_eq!(parse_rule(text), Some(rule), "{text}");
        }
        assert_eq!(parse_rule("когда-нибудь"), None);
    }

    #[test]
    fn next_dates() {
        // 2026-10-30 is a Friday.
        assert_eq!(next_after(Rule::Days(1), d("2026-10-30")), d("2026-10-31"));
        assert_eq!(next_after(Rule::Weekday(Weekday::Mon), d("2026-10-30")), d("2026-11-02"));
        assert_eq!(next_after(Rule::Weekday(Weekday::Fri), d("2026-10-30")), d("2026-11-06"));
        assert_eq!(next_after(Rule::Workdays, d("2026-10-30")), d("2026-11-02"));
        assert_eq!(next_after(Rule::Months { n: 1, day: Some(1) }, d("2026-10-30")), d("2026-11-01"));
        assert_eq!(next_after(Rule::Months { n: 1, day: Some(31) }, d("2026-10-31")), d("2026-11-30"));
        assert_eq!(next_after(Rule::Months { n: 1, day: Some(15) }, d("2026-10-10")), d("2026-10-15"));
        assert_eq!(next_after(Rule::Months { n: 1, day: None }, d("2026-01-31")), d("2026-02-28"));
        assert_eq!(next_after(Rule::Years(1), d("2028-02-29")), d("2029-02-28"));
        // A late completion does not leave the next one in the past.
        assert_eq!(next_from(Rule::Days(7), d("2026-10-01"), d("2026-10-30")), d("2026-11-05"));
    }
}
