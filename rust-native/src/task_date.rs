//! Natural task dates use one explicit clock value and Amsterdam's Gregorian
//! calendar. Only the validated ISO date (or clear token) reaches a script.
use objc2::rc::{autoreleasepool, Retained};
use objc2_foundation::{
    NSCalendar, NSCalendarIdentifierGregorian, NSCalendarOptions, NSCalendarUnit, NSDate,
    NSDateFormatter, NSLocale, NSString, NSTimeZone,
};
use rustcast_core::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskDate {
    pub input: String,
    pub value: Option<String>,
    pub label: String,
}
impl TaskDate {
    pub fn is_valid(&self) -> bool {
        self.value.is_some()
    }
}
pub fn supports(command: &Command) -> bool {
    if command.kind != "script" {
        return false;
    }
    let Some(prompt) = command.input_prompt.as_deref() else {
        return false;
    };
    // Match Swift String.contains at complete Character boundaries, as the
    // existing templates::needs_clipboard demand check does for its own token.
    autoreleasepool(|_| {
        let native = NSString::from_str(prompt);
        let mut scanned = 0;
        let mut utf16 = 0;
        prompt.match_indices("YYYY-MM-DD").any(|(byte, token)| {
            utf16 += prompt[scanned..byte].encode_utf16().count();
            let start = utf16;
            let end = start + token.len();
            scanned = byte + token.len();
            utf16 = end;
            let first = native.rangeOfComposedCharacterSequenceAtIndex(start);
            let last = native.rangeOfComposedCharacterSequenceAtIndex(end - 1);
            first.location == start && last.location + last.length == end
        })
    })
}

fn formatter(
    calendar: &NSCalendar,
    zone: &NSTimeZone,
    locale: &str,
    format: &str,
) -> Retained<NSDateFormatter> {
    let value = NSDateFormatter::new();
    value.setCalendar(Some(calendar));
    value.setTimeZone(Some(zone));
    value.setLocale(Some(&NSLocale::localeWithLocaleIdentifier(
        &NSString::from_str(locale),
    )));
    value.setDateFormat(Some(&NSString::from_str(format)));
    value.setLenient(false);
    value
}
fn relative(text: &str) -> Option<(NSCalendarUnit, isize)> {
    let day = match text {
        "today" | "vandaag" => Some(0),
        "tomorrow" | "morgen" => Some(1),
        "overmorgen" | "day after tomorrow" => Some(2),
        "yesterday" | "gisteren" => Some(-1),
        _ => None,
    };
    if let Some(day) = day {
        return Some((NSCalendarUnit::Day, day));
    }
    match text {
        "next week" | "volgende week" => return Some((NSCalendarUnit::WeekOfYear, 1)),
        "next month" | "volgende maand" => return Some((NSCalendarUnit::Month, 1)),
        "next year" | "volgend jaar" => return Some((NSCalendarUnit::Year, 1)),
        _ => (),
    }
    let parts = text.split(' ').collect::<Vec<_>>();
    if parts.len() != 3
        || !["in", "over"].contains(&parts[0])
        || parts[1].is_empty()
        || parts[1].len() > 4
        || !parts[1].bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let unit = match parts[2] {
        "day" | "days" | "dag" | "dagen" => NSCalendarUnit::Day,
        "week" | "weeks" | "weken" => NSCalendarUnit::WeekOfYear,
        "month" | "months" | "maand" | "maanden" => NSCalendarUnit::Month,
        "year" | "years" | "jaar" | "jaren" => NSCalendarUnit::Year,
        _ => return None,
    };
    Some((unit, parts[1].parse().ok()?))
}
fn weekday(text: &str) -> Option<isize> {
    match text
        .strip_prefix("next ")
        .or_else(|| text.strip_prefix("volgende "))
        .unwrap_or(text)
    {
        "sunday" | "zondag" => Some(1),
        "monday" | "maandag" => Some(2),
        "tuesday" | "dinsdag" => Some(3),
        "wednesday" | "woensdag" => Some(4),
        "thursday" | "donderdag" => Some(5),
        "friday" | "vrijdag" => Some(6),
        "saturday" | "zaterdag" => Some(7),
        _ => None,
    }
}
/// `now` is a frozen Unix timestamp supplied by the editor owner. This function
/// never reads a live clock, so submission can reuse the displayed value.
pub fn resolve(input: &str, now: f64) -> Result<TaskDate, String> {
    if !now.is_finite() {
        return Err("Task date clock must be finite.".into());
    }
    autoreleasepool(|_| {
        let result = |value: Option<String>, label: &str| TaskDate {
            input: input.to_owned(),
            value,
            label: label.to_owned(),
        };
        let text = input
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if ["clear", "wissen", "wis", "verwijderen"].contains(&text.as_str()) {
            return Ok(result(Some("clear".into()), "Datum wissen"));
        }
        if text.is_empty() {
            return Ok(result(None, "Bijv. morgen, next month of in 3 days"));
        }
        let calendar = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
            .ok_or("Gregorian task date calendar is unavailable.")?;
        let zone = NSTimeZone::timeZoneWithName(&NSString::from_str("Europe/Amsterdam"))
            .ok_or("Amsterdam task date time zone is unavailable.")?;
        calendar.setTimeZone(&zone);
        let today = calendar.startOfDayForDate(&NSDate::dateWithTimeIntervalSince1970(now));
        let add = |unit, value, date: &NSDate| {
            calendar.dateByAddingUnit_value_toDate_options(
                unit,
                value,
                date,
                NSCalendarOptions::empty(),
            )
        };
        let mut date = relative(&text).and_then(|(unit, value)| add(unit, value, &today));
        if let Some(weekday) = weekday(&text) {
            let distance =
                (weekday - calendar.component_fromDate(NSCalendarUnit::Weekday, &today) + 7) % 7;
            date = add(
                NSCalendarUnit::Day,
                if distance == 0 { 7 } else { distance },
                &today,
            );
        }
        for format in [
            "yyyy-MM-dd",
            "dd-MM-yyyy",
            "d MMMM yyyy",
            "d MMM yyyy",
            "d MMMM",
            "d MMM",
        ] {
            for locale in ["nl_NL", "en_GB"] {
                if date.is_some() {
                    break;
                }
                let includes_year = format.contains("yyyy");
                let candidate = if includes_year {
                    text.clone()
                } else {
                    format!(
                        "{text} {}",
                        calendar.component_fromDate(NSCalendarUnit::Year, &today)
                    )
                };
                let parse_format = if includes_year {
                    format.to_owned()
                } else {
                    format!("{format} yyyy")
                };
                let parser = formatter(&calendar, &zone, locale, &parse_format);
                if let Some(parsed) = parser.dateFromString(&NSString::from_str(&candidate)) {
                    if parser.stringFromDate(&parsed).to_string().to_lowercase() == candidate {
                        date = if !includes_year
                            && parsed.timeIntervalSince1970() < today.timeIntervalSince1970()
                        {
                            add(NSCalendarUnit::Year, 1, &parsed)
                        } else {
                            Some(parsed)
                        };
                    }
                }
            }
        }
        let Some(date) = date else {
            return Ok(result(None, "Datum niet herkend"));
        };
        if !(1..=9999).contains(&calendar.component_fromDate(NSCalendarUnit::Year, &date)) {
            return Ok(result(None, "Datum niet herkend"));
        }
        let iso = formatter(&calendar, &zone, "en_US_POSIX", "yyyy-MM-dd")
            .stringFromDate(&date)
            .to_string();
        let display = formatter(&calendar, &zone, "nl_NL", "EEEE d MMMM yyyy")
            .stringFromDate(&date)
            .to_string();
        Ok(result(Some(iso), &display))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn instant(value: &str) -> f64 {
        objc2_foundation::NSISO8601DateFormatter::new()
            .dateFromString(&NSString::from_str(value))
            .unwrap()
            .timeIntervalSince1970()
    }
    #[test]
    fn original_natural_date_vectors_and_labels_match() {
        let now = instant("2026-10-26T12:00:00Z");
        for (input, expected) in [
            ("tomorrow", "2026-10-27"),
            ("morgen", "2026-10-27"),
            ("next month", "2026-11-26"),
            ("in 3 days", "2026-10-29"),
            ("over 3 dagen", "2026-10-29"),
            ("volgende week", "2026-11-02"),
            ("next monday", "2026-11-02"),
            ("vrijdag", "2026-10-30"),
            ("clear", "clear"),
            ("wissen", "clear"),
            ("wis", "clear"),
            ("verwijderen", "clear"),
            ("2026-11-05", "2026-11-05"),
            ("05-11-2026", "2026-11-05"),
            ("5 november", "2026-11-05"),
            ("5 Nov", "2026-11-05"),
            ("5 November 2027", "2027-11-05"),
            ("  MORGEN  ", "2026-10-27"),
            ("in\u{a0}3\t dagen", "2026-10-29"),
            ("in 0003 days", "2026-10-29"),
            ("in 0 days", "2026-10-26"),
            ("yesterday", "2026-10-25"),
            ("volgende maandag", "2026-11-02"),
            ("1 januari", "2027-01-01"),
        ] {
            let date = resolve(input, now).unwrap();
            assert_eq!(date.value.as_deref(), Some(expected), "{input}");
            assert_eq!(date.input, input);
            assert!(date.is_valid());
        }
        assert_eq!(
            resolve("morgen", now).unwrap().label,
            "dinsdag 27 oktober 2026"
        );
        assert_eq!(resolve("clear", now).unwrap().label, "Datum wissen");
        assert_eq!(
            resolve("", now).unwrap().label,
            "Bijv. morgen, next month of in 3 days"
        );
    }
    #[test]
    fn calendar_month_year_and_amsterdam_dst_boundaries_are_not_fixed_seconds() {
        for (now, input, expected) in [
            ("2026-01-31T12:00:00Z", "next month", "2026-02-28"),
            ("2026-03-28T12:00:00Z", "tomorrow", "2026-03-29"),
            ("2026-12-31T23:30:00Z", "today", "2027-01-01"),
            ("2026-03-28T23:30:00Z", "today", "2026-03-29"),
            ("2026-03-28T23:30:00Z", "tomorrow", "2026-03-30"),
            ("2026-10-24T22:30:00Z", "tomorrow", "2026-10-26"),
            ("2024-02-29T12:00:00Z", "next year", "2025-02-28"),
        ] {
            assert_eq!(
                resolve(input, instant(now)).unwrap().value.as_deref(),
                Some(expected),
                "{now}: {input}"
            );
        }
    }
    #[test]
    fn strict_absolute_roundtrip_year_bounds_invalid_and_frozen_clock() {
        let now = instant("2026-10-26T12:00:00Z");
        for input in [
            "",
            "in 3",
            "2026-02-30",
            "2026-13-01",
            "do something tomorrow",
            "in -3 days",
            "in 10000 days",
            "2026-2-03",
            "5-11-2026",
            "05 november 2026",
            "5 november 2026 trailing",
            "29 februari",
            "0000-01-01",
            "10000-01-01",
            "in 9999 years",
        ] {
            let value = resolve(input, now).unwrap();
            assert!(!value.is_valid(), "{input}");
            if !input.is_empty() {
                assert_eq!(value.label, "Datum niet herkend");
            }
        }
        for input in ["0001-01-01", "9999-12-31"] {
            assert_eq!(resolve(input, now).unwrap().value.as_deref(), Some(input));
        }
        for now in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(resolve("morgen", now).is_err());
        }
        let displayed = resolve("morgen", 946_728_000.0).unwrap();
        assert_eq!(displayed.value.as_deref(), Some("2000-01-02"));
        assert_eq!(displayed, resolve("morgen", 946_728_000.0).unwrap());
    }
    #[test]
    fn only_script_prompts_with_the_original_case_sensitive_hint_opt_in() {
        let command = |kind: &str, prompt: Option<&str>| {
            serde_json::from_value::<Command>(serde_json::json!({
            "id":"fixture.date","title":"Date","kind":kind,"target":"/bin/echo","inputPrompt":prompt
        })).unwrap()
        };
        assert!(supports(&command(
            "script",
            Some("Date (YYYY-MM-DD or clear)")
        )));
        assert!(!supports(&command("script", Some("Text"))));
        assert!(supports(&command("script", Some("😀 YYYY-MM-DD"))));
        assert!(!supports(&command(
            "script",
            Some("Date YYYY-MM-DD\u{0301}")
        )));
        assert!(supports(&command(
            "script",
            Some("YYYY-MM-DD\u{0301} or YYYY-MM-DD")
        )));
        assert!(!supports(&command("script", Some("yyyy-mm-dd"))));
        assert!(!supports(&command("script", None)));
        assert!(!supports(&command("url", Some("YYYY-MM-DD"))));
    }
}
