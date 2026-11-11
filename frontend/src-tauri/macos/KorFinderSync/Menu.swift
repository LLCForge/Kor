// Kor's Finder context-menu entries (§25.1, §25.2, §27.6; ADR 0052). Pure logic, shared by the
// Finder Sync extension and Tests/main.swift. Mirrors `kor-shellext/src/menu.rs` (Windows 11);
// a Rust test checks that the titles here match.

import Foundation

enum Lang {
    case tr, en, fr

    /// Kor's `ui.language` (`tr`, `en`, `fr`; `auto`/unset follows the system languages).
    static func pick(config: String?, preferred: [String]) -> Lang {
        switch config?.trimmingCharacters(in: .whitespaces) {
        case "tr": return .tr
        case "en": return .en
        case "fr": return .fr
        default:
            let first = preferred.first?.lowercased() ?? "en"
            if first.hasPrefix("tr") { return .tr }
            if first.hasPrefix("fr") { return .fr }
            return .en
        }
    }
}

enum Verb: Int, CaseIterable {
    case terminal = 1, desktop, edit, explain, bugs, test

    func title(_ l: Lang) -> String {
        switch (self, l) {
        case (.terminal, .tr): return "Kor CLI ile aç"
        case (.terminal, .en): return "Open with Kor CLI"
        case (.terminal, .fr): return "Ouvrir avec Kor CLI"
        case (.desktop, .tr): return "Kor Desktop ile aç"
        case (.desktop, .en): return "Open with Kor Desktop"
        case (.desktop, .fr): return "Ouvrir avec Kor Desktop"
        case (.edit, .tr): return "Kor ile düzenle"
        case (.edit, .en): return "Edit with Kor"
        case (.edit, .fr): return "Modifier avec Kor"
        case (.explain, .tr): return "Kor: Projeyi açıkla"
        case (.explain, .en): return "Kor: Explain the project"
        case (.explain, .fr): return "Kor : Expliquer le projet"
        case (.bugs, .tr): return "Kor: Hata bul"
        case (.bugs, .en): return "Kor: Find bugs"
        case (.bugs, .fr): return "Kor : Trouver les bogues"
        case (.test, .tr): return "Kor: Bu dosyayı test et"
        case (.test, .en): return "Kor: Test this file"
        case (.test, .fr): return "Kor : Tester ce fichier"
        }
    }

    /// Shown for `count` selected items of which `files` are files; `extended` is
    /// `shell.context_menu.extended`.
    func visible(count: Int, files: Int, extended: Bool) -> Bool {
        if count == 0 { return false }
        let filesOnly = files == count
        switch self {
        case .terminal, .desktop: return true
        case .edit: return filesOnly
        case .explain, .bugs: return extended
        case .test: return extended && filesOnly
        }
    }

    /// `kor-desktop` arguments (`kor_runtime::osint` protocol), at most 256 paths.
    func args(_ paths: [String]) -> [String] {
        let head: [String]
        switch self {
        case .terminal: head = ["--terminal"]
        case .desktop: head = ["--open"]
        case .edit: head = ["--edit"]
        case .explain: head = ["--task", "explain"]
        case .bugs: head = ["--task", "bugs"]
        case .test: head = ["--task", "test"]
        }
        return head + paths.prefix(256)
    }
}

/// `ui.language` and `shell.context_menu.extended` from Kor's `config.toml` (only the two keys
/// the menu needs; anything else is ignored).
struct Settings: Equatable {
    var language: String?
    var extended = false

    static func parse(_ toml: String) -> Settings {
        var s = Settings()
        var section = ""
        for raw in toml.split(whereSeparator: \.isNewline) {
            let line = raw.split(separator: "#", maxSplits: 1, omittingEmptySubsequences: false).first
                .map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
            if line.hasPrefix("[") && line.hasSuffix("]") {
                section = String(line.dropFirst().dropLast()).trimmingCharacters(in: .whitespaces)
                continue
            }
            let kv = line.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            guard kv.count == 2 else { continue }
            let key = section.isEmpty ? kv[0] : "\(section).\(kv[0])"
            let value = kv[1].trimmingCharacters(in: CharacterSet(charactersIn: "\"'"))
            switch key {
            case "ui.language": s.language = value
            case "shell.context_menu.extended": s.extended = value == "true"
            default: break
            }
        }
        return s
    }

    /// `~/Library/Application Support/kor/config.toml` under the real home (`home` is passed in
    /// because a sandboxed extension's `NSHomeDirectory()` is its container).
    static func load(home: String) -> Settings {
        let path = (home as NSString).appendingPathComponent("Library/Application Support/kor/config.toml")
        guard let text = try? String(contentsOfFile: path, encoding: .utf8) else { return Settings() }
        return parse(text)
    }
}
