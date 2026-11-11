// Kor top-bar status for GNOME Shell 45+ (Master Build Prompt §27.10, ADR 0052).
// Installed by `kor os-integration install gnome-shell`; the `kor` path below is filled in then.
// Everything shown comes from `kor os-integration feed` (JSON, already localized); actions open
// kor:// links (handled by the Kor desktop app) or run `kor os-integration approve --id <id>`.
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import Gio from 'gi://Gio';
import St from 'gi://St';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

const KOR = @KOR_JSON@;
const REFRESH_SECONDS = 30;

function run(argv, done) {
    try {
        const proc = Gio.Subprocess.new(argv, Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE);
        proc.communicate_utf8_async(null, null, (p, res) => {
            try {
                const [, out] = p.communicate_utf8_finish(res);
                done(out ?? '');
            } catch (e) {
                done('');
            }
        });
    } catch (e) {
        done('');
    }
}

function openUri(uri) {
    try {
        Gio.AppInfo.launch_default_for_uri(uri, null);
    } catch (e) {
        logError(e, 'Kor: cannot open link');
    }
}

export default class KorExtension extends Extension {
    enable() {
        this._button = new PanelMenu.Button(0.0, 'Kor');
        this._label = new St.Label({text: 'Kor', y_align: Clutter.ActorAlign.CENTER});
        this._button.add_child(this._label);
        Main.panel.addToStatusArea(this.uuid, this._button);
        this._refresh();
        this._timer = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, REFRESH_SECONDS, () => {
            this._refresh();
            return GLib.SOURCE_CONTINUE;
        });
    }

    disable() {
        if (this._timer) {
            GLib.source_remove(this._timer);
            this._timer = null;
        }
        this._button?.destroy();
        this._button = null;
        this._label = null;
    }

    _refresh() {
        run([KOR, 'os-integration', 'feed'], (out) => {
            if (!this._button)
                return;
            let feed = null;
            try {
                feed = JSON.parse(out);
            } catch (e) {
                feed = null;
            }
            this._render(feed);
        });
    }

    _render(feed) {
        const menu = this._button.menu;
        menu.removeAll();
        if (!feed) {
            this._label.text = 'Kor';
            menu.addMenuItem(new PopupMenu.PopupMenuItem('kor ?', {reactive: false}));
            return;
        }
        const l = feed.labels;
        const waiting = feed.waiting ?? [];
        this._label.text = waiting.length > 0 ? `Kor • ${waiting.length}` : 'Kor';
        menu.addMenuItem(new PopupMenu.PopupMenuItem(feed.status, {reactive: false}));
        const add = (text, fn) => {
            const item = new PopupMenu.PopupMenuItem(text);
            item.connect('activate', fn);
            menu.addMenuItem(item);
        };
        add(l.newTask, () => openUri(feed.newTask));
        if (waiting.length > 0) {
            menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem(l.waiting));
            for (const w of waiting)
                add(`${l.approve}: ${w.summary}`, () => run([KOR, 'os-integration', 'approve', '--id', w.id], () => this._refresh()));
        }
        if ((feed.projects ?? []).length > 0) {
            menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem(l.projects));
            for (const p of feed.projects)
                add(p.name, () => openUri(p.url));
        }
        if ((feed.sessions ?? []).length > 0) {
            menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem(l.sessions));
            for (const s of feed.sessions)
                add(s.title, () => openUri(s.url));
        }
    }
}
