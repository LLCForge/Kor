// Kor's Finder Sync extension (§25.2; ADR 0052): "Kor CLI ile aç", "Kor Desktop ile aç",
// "Kor ile düzenle" and, with `shell.context_menu.extended`, the quick tasks — on folders, files,
// multiple selections and the empty space of a Finder window. Each entry starts Kor.app with the
// `kor_runtime::osint` arguments; that second instance hands them to the running window
// (single-instance plugin) and exits.

import Cocoa
import FinderSync

@objc(KorFinderSync)
final class KorFinderSync: FIFinderSync {
    override init() {
        super.init()
        // Every folder: the menu is offered everywhere, like the Windows and Linux entries.
        FIFinderSyncController.default().directoryURLs = [URL(fileURLWithPath: "/")]
    }

    /// The real home (inside the sandbox `NSHomeDirectory()` is the extension's container).
    private var realHome: String {
        if let pw = getpwuid(getuid()), let dir = pw.pointee.pw_dir { return String(cString: dir) }
        return NSHomeDirectory()
    }

    private func selection(_ kind: FIMenuKind) -> [URL] {
        let c = FIFinderSyncController.default()
        if kind == .contextualMenuForContainer { return c.targetedURL().map { [$0] } ?? [] }
        return c.selectedItemURLs() ?? []
    }

    private func isDir(_ url: URL) -> Bool {
        (try? url.resourceValues(forKeys: [.isDirectoryKey]).isDirectory) == true
    }

    override func menu(for menuKind: FIMenuKind) -> NSMenu? {
        guard menuKind == .contextualMenuForItems || menuKind == .contextualMenuForContainer else { return nil }
        let urls = selection(menuKind).filter(\.isFileURL)
        let settings = Settings.load(home: realHome)
        let lang = Lang.pick(config: settings.language, preferred: Locale.preferredLanguages)
        let files = urls.filter { !isDir($0) }.count
        let menu = NSMenu(title: "")
        for verb in Verb.allCases where verb.visible(count: urls.count, files: files, extended: settings.extended) {
            let item = NSMenuItem(title: verb.title(lang), action: #selector(run(_:)), keyEquivalent: "")
            item.tag = verb.rawValue
            menu.addItem(item)
        }
        return menu
    }

    @objc private func run(_ sender: NSMenuItem) {
        guard let verb = Verb(rawValue: sender.tag) else { return }
        let c = FIFinderSyncController.default()
        var urls = c.selectedItemURLs() ?? []
        if urls.isEmpty, let target = c.targetedURL() { urls = [target] }
        let paths = urls.filter(\.isFileURL).map(\.path)
        guard !paths.isEmpty else { return }
        // Kor.app/Contents/PlugIns/KorFinderSync.appex → Kor.app
        let app = Bundle.main.bundleURL
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let config = NSWorkspace.OpenConfiguration()
        config.arguments = verb.args(paths)
        config.createsNewApplicationInstance = true
        config.activates = true
        NSWorkspace.shared.openApplication(at: app, configuration: config) { _, error in
            if let error = error { NSLog("Kor Finder extension: %@", error.localizedDescription) }
        }
    }
}
