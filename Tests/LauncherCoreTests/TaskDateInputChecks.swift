import Foundation
import LauncherCore
import LauncherModels

func checkTaskDateInput() {
    let now = ISO8601DateFormatter().date(from: "2026-10-26T12:00:00Z")!
    for (input, expected) in [("tomorrow", "2026-10-27"), ("morgen", "2026-10-27"), ("next month", "2026-11-26"), ("in 3 days", "2026-10-29"), ("over 3 dagen", "2026-10-29"), ("volgende week", "2026-11-02"), ("next monday", "2026-11-02"), ("vrijdag", "2026-10-30"), ("clear", "clear"), ("wissen", "clear"), ("2026-11-05", "2026-11-05"), ("05-11-2026", "2026-11-05"), ("5 november", "2026-11-05"), ("  MORGEN  ", "2026-10-27")] {
        precondition(TaskDateInput.resolve(input, now: now).value == expected, input)
    }
    for input in ["", "in 3", "2026-02-30", "2026-13-01", "do something tomorrow", "in -3 days", "in 10000 days"] {
        precondition(!TaskDateInput.resolve(input, now: now).isValid, input)
    }
    for (instant, input, expected) in [("2026-01-31T12:00:00Z", "next month", "2026-02-28"), ("2026-03-28T12:00:00Z", "tomorrow", "2026-03-29"), ("2026-12-31T23:30:00Z", "today", "2027-01-01")] {
        precondition(TaskDateInput.resolve(input, now: ISO8601DateFormatter().date(from: instant)!).value == expected)
    }
    precondition(TaskDateInput.resolve("morgen", now: now).label == "dinsdag 27 oktober 2026")
    let command = Command(id: "fixture.date", title: "Date", kind: "script", target: "/bin/echo", inputPrompt: "Date (YYYY-MM-DD or clear)")
    precondition(TaskDateInput.supports(command))
    precondition(!TaskDateInput.supports(Command(id: "fixture.text", title: "Text", kind: "script", target: "/bin/echo", inputPrompt: "Text")))
    print("Natural date checks passed")
}
