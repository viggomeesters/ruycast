import Foundation

/// The ISO-date hint in existing script prompts opts into natural date entry.
/// Scripts still receive only a validated ISO date or the existing clear token.
public struct TaskDateInput: Equatable {
    public let input: String
    public let value: String?
    public let label: String
    public var isValid: Bool { value != nil }

    public static func supports(_ command: Command) -> Bool {
        command.kind == "script" && command.inputPrompt?.contains("YYYY-MM-DD") == true
    }

    public static func resolve(_ input: String, now: Date = Date()) -> TaskDateInput {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "Europe/Amsterdam")!
        let today = calendar.startOfDay(for: now)
        let text = input.lowercased().split(whereSeparator: { $0.isWhitespace }).joined(separator: " ")
        func result(_ value: String?, _ label: String) -> TaskDateInput {
            TaskDateInput(input: input, value: value, label: label)
        }
        if ["clear", "wissen", "wis", "verwijderen"].contains(text) { return result("clear", "Datum wissen") }
        if text.isEmpty { return result(nil, "Bijv. morgen, next month of in 3 days") }
        var date: Date?
        let offsets = ["today": 0, "vandaag": 0, "tomorrow": 1, "morgen": 1, "overmorgen": 2, "day after tomorrow": 2, "yesterday": -1, "gisteren": -1]
        if let offset = offsets[text] { date = calendar.date(byAdding: .day, value: offset, to: today) }
        let next: [String: Calendar.Component] = ["next week": .weekOfYear, "volgende week": .weekOfYear, "next month": .month, "volgende maand": .month, "next year": .year, "volgend jaar": .year]
        if let component = next[text] { date = calendar.date(byAdding: component, value: 1, to: today) }
        if let match = text.range(of: #"^(?:in|over) [0-9]{1,4} (?:days?|dagen|dag|weeks?|weken|week|months?|maanden|maand|years?|jaren|jaar)$"#, options: .regularExpression), match == text.startIndex..<text.endIndex {
            let parts = text.split(separator: " ")
            let count = Int(parts[1])!
            let unit = String(parts[2])
            let component: Calendar.Component = ["day", "days", "dag", "dagen"].contains(unit) ? .day : ["week", "weeks", "weken"].contains(unit) ? .weekOfYear : ["month", "months", "maand", "maanden"].contains(unit) ? .month : .year
            date = calendar.date(byAdding: component, value: count, to: today)
        }
        let weekdays = ["sunday": 1, "zondag": 1, "monday": 2, "maandag": 2, "tuesday": 3, "dinsdag": 3, "wednesday": 4, "woensdag": 4, "thursday": 5, "donderdag": 5, "friday": 6, "vrijdag": 6, "saturday": 7, "zaterdag": 7]
        let weekdayText = text.replacingOccurrences(of: #"^(next |volgende )"#, with: "", options: .regularExpression)
        if let weekday = weekdays[weekdayText] {
            let distance = (weekday - calendar.component(.weekday, from: today) + 7) % 7
            date = calendar.date(byAdding: .day, value: distance == 0 ? 7 : distance, to: today)
        }
        // Parse complete absolute dates strictly; DateFormatter alone can accept
        // partial input or roll impossible calendar dates into a different month.
        for format in ["yyyy-MM-dd", "dd-MM-yyyy", "d MMMM yyyy", "d MMM yyyy", "d MMMM", "d MMM"] where date == nil {
            for locale in ["nl_NL", "en_GB"] where date == nil {
                let formatter = DateFormatter()
                formatter.calendar = calendar; formatter.timeZone = calendar.timeZone
                formatter.locale = Locale(identifier: locale); formatter.dateFormat = format
                formatter.isLenient = false
                let includesYear = format.contains("yyyy")
                let candidate = includesYear ? text : text + " " + String(calendar.component(.year, from: today))
                let parseFormat = includesYear ? format : format + " yyyy"
                formatter.dateFormat = parseFormat
                if let parsed = formatter.date(from: candidate), formatter.string(from: parsed).lowercased() == candidate {
                    date = parsed
                    if !includesYear, parsed < today { date = calendar.date(byAdding: .year, value: 1, to: parsed) }
                }
            }
        }
        guard let date, (1...9999).contains(calendar.component(.year, from: date)) else { return result(nil, "Datum niet herkend") }
        let iso = DateFormatter(); iso.calendar = calendar; iso.timeZone = calendar.timeZone
        iso.locale = Locale(identifier: "en_US_POSIX"); iso.dateFormat = "yyyy-MM-dd"
        let display = DateFormatter(); display.calendar = calendar; display.timeZone = calendar.timeZone
        display.locale = Locale(identifier: "nl_NL"); display.dateFormat = "EEEE d MMMM yyyy"
        return result(iso.string(from: date), display.string(from: date))
    }
}
