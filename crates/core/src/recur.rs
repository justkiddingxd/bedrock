//! Repeating tasks and reminders: `🔁 каждый день`, `🔁 по понедельникам`,
//! `🔁 1-го числа`, `🔁 every week`.
//!
//! The line keeps the first time (`⏰ 2026-11-03 09:00 🔁 каждый день`) and every
//! later time follows from the rule, so notes do not change by themselves. When
//! a repeating task is done, its next occurrence is added as a new task.

use std::sync::LazyLock;

use chrono::{Datelike, Duration, Months, NaiveDate, NaiveDateTime, Weekday};
use regex::Regex;

/// The rule after 🔁, up to the next task marker or tag.
pub static RECUR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"🔁\x{FE0F}?\s*([^📅⏳✅🛫➕⏰⏫🔼🔽🔺⏬#\n]+)").unwrap());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    Days(u32),
    /// On these days of the week (bit 0 is Monday).
    Weekly(u8),
    /// Every `n` months, on `day` (the same day as the first time when `None`).
    Months { n: u32, day: Option<u32> },
    Years(u32),
}

const fn bit(w: Weekday) -> u8 {
    1 << w.num_days_from_monday()
}

const WORKDAYS: u8 = 0b0001_1111;
const WEEKEND: u8 = 0b0110_0000;

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,3})").unwrap());
static MONTH_DAY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(\d{1,2})\s*(?:-?(?:го|е|ое))?\s*числ|(\d{1,2})-(?:го|е|ое)|on the (\d{1,2})(?:st|nd|rd|th)?").unwrap()
});

const WEEKDAYS: [(&str, &str, Weekday); 7] = [
    ("понедельн", "monday", Weekday::Mon),
    ("вторн", "tuesday", Weekday::Tue),
    ("сред", "wednesday", Weekday::Wed),
    ("четверг", "thursday", Weekday::Thu),
    ("пятниц", "friday", Weekday::Fri),
    ("суббот", "saturday", Weekday::Sat),
    ("воскресен", "sunday", Weekday::Sun),
];

/// Understands Russian («каждый день», «ежемесячно», «каждые 2 недели», «раз в
/// неделю», «по вторникам и четвергам», «по будням», «1-го числа», «каждый месяц
/// 5-го», «каждый год») and English (Obsidian Tasks style: «every week», «every
/// month on the 1st»).
pub fn parse_rule(text: &str) -> Option<Rule> {
    let t = text.trim().to_lowercase();
    if let Some(c) = MONTH_DAY.captures(&t) {
        let day = c.get(1).or(c.get(2)).or(c.get(3))?.as_str().parse::<u32>().ok().filter(|d| (1..=31).contains(d))?;
        return Some(Rule::Months { n: 1, day: Some(day) });
    }
    if t.contains("будн") || t.contains("weekday") || t.contains("рабоч") {
        return Some(Rule::Weekly(WORKDAYS));
    }
    if t.contains("выходн") || t.contains("weekend") {
        return Some(Rule::Weekly(WEEKEND));
    }
    let days = WEEKDAYS.iter().filter(|(ru, en, _)| t.contains(ru) || t.contains(en)).fold(0, |mask, (_, _, w)| mask | bit(*w));
    if days != 0 {
        return Some(Rule::Weekly(days));
    }
    let n = NUMBER.captures(&t).and_then(|c| c[1].parse::<u32>().ok()).filter(|n| *n > 0);
    let unit = |ru: &[&str], en: &[&str]| ru.iter().any(|s| t.contains(s)) || en.iter().any(|s| t.contains(s));
    // Years first: «ежегодно» has «дн» in it.
    if unit(&["год", "лет"], &["year"]) {
        return Some(Rule::Years(n.unwrap_or(1)));
    }
    if unit(&["дн", "день"], &["day", "daily"]) {
        return Some(Rule::Days(n.unwrap_or(1)));
    }
    if unit(&["недел"], &["week"]) {
        return Some(Rule::Days(7 * n.unwrap_or(1)));
    }
    if unit(&["месяц", "ежемесячн"], &["month"]) {
        return Some(Rule::Months { n: n.unwrap_or(1), day: None });
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
        Rule::Weekly(days) => {
            let mut d = base + Duration::days(1);
            while days & bit(d.weekday()) == 0 {
                d += Duration::days(1);
            }
            d
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

/// The days of a rule from `first` on, `first` included. Months and years count
/// from `first`: from the 31st a monthly rule takes the last day of a shorter
/// month and comes back to the 31st.
pub fn occurrences(rule: Rule, first: NaiveDate) -> impl Iterator<Item = NaiveDate> {
    let mut step = 0;
    let mut current = Some(first);
    std::iter::from_fn(move || {
        let day = current?;
        step += 1;
        current = match rule {
            Rule::Months { n, day: None } => first.checked_add_months(Months::new(n.saturating_mul(step))),
            Rule::Years(n) => first.checked_add_months(Months::new(n.saturating_mul(12).saturating_mul(step))),
            _ => Some(next_after(rule, day)),
        };
        Some(day)
    })
}

/// The first occurrence after `base` that is not before `from`.
pub fn next_from(rule: Rule, base: NaiveDate, from: NaiveDate) -> NaiveDate {
    occurrences(rule, base).skip(1).find(|d| *d >= from).unwrap_or(from)
}

/// The times of a repeating reminder whose first time is `first`.
pub fn times(rule: Rule, first: NaiveDateTime) -> impl Iterator<Item = NaiveDateTime> {
    occurrences(rule, first.date()).map(move |d| d.and_time(first.time()))
}

/// The last time at or before `now`, if it has started.
pub fn last_time(rule: Rule, first: NaiveDateTime, now: NaiveDateTime) -> Option<NaiveDateTime> {
    times(rule, first).take_while(|t| *t <= now).last()
}

/// The first time after `now`.
pub fn next_time(rule: Rule, first: NaiveDateTime, now: NaiveDateTime) -> Option<NaiveDateTime> {
    times(rule, first).find(|t| *t > now)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn rules() {
        let cases = [
            ("каждый день", Rule::Days(1)),
            ("ежедневно", Rule::Days(1)),
            ("раз в день", Rule::Days(1)),
            ("каждые 3 дня", Rule::Days(3)),
            ("каждую неделю", Rule::Days(7)),
            ("еженедельно", Rule::Days(7)),
            ("раз в неделю", Rule::Days(7)),
            ("каждые 2 недели", Rule::Days(14)),
            ("каждый понедельник", Rule::Weekly(bit(Weekday::Mon))),
            ("по пятницам", Rule::Weekly(bit(Weekday::Fri))),
            ("каждую среду", Rule::Weekly(bit(Weekday::Wed))),
            ("каждое воскресенье", Rule::Weekly(bit(Weekday::Sun))),
            ("каждую неделю в пятницу", Rule::Weekly(bit(Weekday::Fri))),
            ("по вторникам и четвергам", Rule::Weekly(bit(Weekday::Tue) | bit(Weekday::Thu))),
            ("каждый будний день", Rule::Weekly(WORKDAYS)),
            ("по будням", Rule::Weekly(WORKDAYS)),
            ("по выходным", Rule::Weekly(WEEKEND)),
            ("каждый месяц", Rule::Months { n: 1, day: None }),
            ("ежемесячно", Rule::Months { n: 1, day: None }),
            ("раз в месяц", Rule::Months { n: 1, day: None }),
            ("каждые 3 месяца", Rule::Months { n: 3, day: None }),
            ("1-го числа", Rule::Months { n: 1, day: Some(1) }),
            ("каждое 15 число", Rule::Months { n: 1, day: Some(15) }),
            ("каждый месяц 5-го", Rule::Months { n: 1, day: Some(5) }),
            ("ежемесячно 20 числа", Rule::Months { n: 1, day: Some(20) }),
            ("каждый год", Rule::Years(1)),
            ("ежегодно", Rule::Years(1)),
            ("every week", Rule::Days(7)),
            ("every 2 days", Rule::Days(2)),
            ("every monday", Rule::Weekly(bit(Weekday::Mon))),
            ("every weekday", Rule::Weekly(WORKDAYS)),
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
        assert_eq!(next_after(Rule::Weekly(bit(Weekday::Mon)), d("2026-10-30")), d("2026-11-02"));
        assert_eq!(next_after(Rule::Weekly(bit(Weekday::Fri)), d("2026-10-30")), d("2026-11-06"));
        assert_eq!(next_after(Rule::Weekly(WORKDAYS), d("2026-10-30")), d("2026-11-02"));
        assert_eq!(next_after(Rule::Months { n: 1, day: Some(1) }, d("2026-10-30")), d("2026-11-01"));
        assert_eq!(next_after(Rule::Months { n: 1, day: Some(31) }, d("2026-10-31")), d("2026-11-30"));
        assert_eq!(next_after(Rule::Months { n: 1, day: Some(15) }, d("2026-10-10")), d("2026-10-15"));
        assert_eq!(next_after(Rule::Months { n: 1, day: None }, d("2026-01-31")), d("2026-02-28"));
        assert_eq!(next_after(Rule::Years(1), d("2028-02-29")), d("2029-02-28"));
        // A late completion does not leave the next one in the past.
        assert_eq!(next_from(Rule::Days(7), d("2026-10-01"), d("2026-10-30")), d("2026-11-05"));
    }

    #[test]
    fn times_of_a_reminder() {
        // Monthly from the 31st: shorter months take their last day, then back to the 31st.
        let monthly: Vec<_> = occurrences(Rule::Months { n: 1, day: None }, d("2026-01-31")).take(4).collect();
        assert_eq!(monthly, [d("2026-01-31"), d("2026-02-28"), d("2026-03-31"), d("2026-04-30")]);

        let daily = Rule::Days(1);
        let first = at("2026-11-01 09:00");
        // Before the first time: nothing has come yet, the first time is next.
        assert_eq!(last_time(daily, first, at("2026-10-31 12:00")), None);
        assert_eq!(next_time(daily, first, at("2026-10-31 12:00")), Some(first));
        // Exactly on time, then a day later and a bit.
        assert_eq!(last_time(daily, first, first), Some(first));
        assert_eq!(next_time(daily, first, first), Some(at("2026-11-02 09:00")));
        assert_eq!(last_time(daily, first, at("2026-11-03 10:30")), Some(at("2026-11-03 09:00")));
        assert_eq!(next_time(daily, first, at("2026-11-03 10:30")), Some(at("2026-11-04 09:00")));
        // Weekly: the same weekday; on weekdays of a rule: the first time stands even if off-rule.
        assert_eq!(next_time(Rule::Days(7), first, at("2026-11-02 00:00")), Some(at("2026-11-08 09:00")));
        let tue_thu = Rule::Weekly(bit(Weekday::Tue) | bit(Weekday::Thu));
        assert_eq!(times(tue_thu, first).take(3).collect::<Vec<_>>(), [first, at("2026-11-03 09:00"), at("2026-11-05 09:00")]);
    }
}
