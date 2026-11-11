// Run on macOS CI: `swiftc Menu.swift Tests/main.swift -o /tmp/kor-menu-tests && /tmp/kor-menu-tests`.

import Foundation

var failures = 0
func check(_ ok: Bool, _ what: String, line: Int = #line) {
    if !ok {
        failures += 1
        print("FAIL line \(line): \(what)")
    }
}

check(Lang.pick(config: "fr", preferred: ["tr-TR"]) == .fr, "config wins")
check(Lang.pick(config: "auto", preferred: ["tr-TR"]) == .tr, "auto follows the system")
check(Lang.pick(config: nil, preferred: []) == .en, "default English")

func shown(_ count: Int, _ files: Int, _ ext: Bool) -> [Verb] {
    Verb.allCases.filter { $0.visible(count: count, files: files, extended: ext) }
}
check(shown(1, 0, false) == [.terminal, .desktop], "folder")
check(shown(3, 3, false) == [.terminal, .desktop, .edit], "files")
check(shown(2, 1, false) == [.terminal, .desktop], "mixed")
check(shown(1, 0, true) == [.terminal, .desktop, .explain, .bugs], "folder extended")
check(shown(2, 2, true) == Verb.allCases, "files extended")
check(shown(0, 0, true).isEmpty, "nothing selected")

check(Verb.edit.args(["/a b/x", "/y"]) == ["--edit", "/a b/x", "/y"], "edit args")
check(Verb.test.args(["/x"]) == ["--task", "test", "/x"], "task args")
check(Verb.terminal.args(Array(repeating: "/p", count: 300)).count == 257, "path cap")

let parsed = Settings.parse("# comment\n[ui]\nlanguage = \"fr\" # trailing\n[shell.context_menu]\nextended = true\n")
check(parsed == Settings(language: "fr", extended: true), "settings \(parsed)")
check(Settings.parse("garbage [") == Settings(), "lenient")
check(Settings.load(home: "/nonexistent") == Settings(), "missing file")

if failures > 0 {
    print("\(failures) failure(s)")
    exit(1)
}
print("Kor Finder menu tests: ok")
