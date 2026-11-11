// Kor status widget for KDE Plasma 6 (Master Build Prompt §27.10, ADR 0052).
// Installed by `kor os-integration install plasmoid`; the `kor` command below is filled in then.
// Data: `kor os-integration feed` (JSON, already localized). Actions open kor:// links or run
// `kor os-integration approve --id <id>`.
import QtQuick
import QtQuick.Layouts
import org.kde.plasma.plasmoid
import org.kde.plasma.components as PlasmaComponents
import org.kde.plasma.plasma5support as P5Support

PlasmoidItem {
    id: root
    readonly property string kor: @KOR_JSON@
    property var feed: null

    function quote(s) { return "'" + s.replace(/'/g, "'\\''") + "'" }
    function refresh() { exec.connectSource(quote(kor) + " os-integration feed") }

    P5Support.DataSource {
        id: exec
        engine: "executable"
        connectedSources: []
        onNewData: (source, data) => {
            if (source.indexOf(" os-integration feed") >= 0) {
                try { root.feed = JSON.parse(data["stdout"]) } catch (e) { root.feed = null }
            } else {
                root.refresh()
            }
            disconnectSource(source)
        }
    }

    Timer {
        interval: 30000
        running: true
        repeat: true
        triggeredOnStart: true
        onTriggered: root.refresh()
    }

    toolTipMainText: "Kor"
    toolTipSubText: root.feed ? root.feed.status : ""

    compactRepresentation: PlasmaComponents.Label {
        text: root.feed && root.feed.waiting.length > 0 ? "Kor • " + root.feed.waiting.length : "Kor"
        MouseArea { anchors.fill: parent; onClicked: root.expanded = !root.expanded }
    }

    fullRepresentation: ColumnLayout {
        Layout.minimumWidth: 280
        PlasmaComponents.Label {
            text: root.feed ? root.feed.status : "kor ?"
            Layout.fillWidth: true
            wrapMode: Text.Wrap
        }
        PlasmaComponents.Button {
            visible: root.feed !== null
            text: root.feed ? root.feed.labels.newTask : ""
            onClicked: Qt.openUrlExternally(root.feed.newTask)
        }
        Repeater {
            model: root.feed ? root.feed.waiting : []
            delegate: PlasmaComponents.Button {
                required property var modelData
                text: root.feed.labels.approve + ": " + modelData.summary
                onClicked: exec.connectSource(root.quote(root.kor) + " os-integration approve --id " + root.quote(modelData.id))
            }
        }
        PlasmaComponents.Label {
            visible: root.feed !== null && root.feed.projects.length > 0
            text: root.feed ? root.feed.labels.projects : ""
            font.bold: true
        }
        Repeater {
            model: root.feed ? root.feed.projects : []
            delegate: PlasmaComponents.ItemDelegate {
                required property var modelData
                text: modelData.name
                Layout.fillWidth: true
                onClicked: Qt.openUrlExternally(modelData.url)
            }
        }
        PlasmaComponents.Label {
            visible: root.feed !== null && root.feed.sessions.length > 0
            text: root.feed ? root.feed.labels.sessions : ""
            font.bold: true
        }
        Repeater {
            model: root.feed ? root.feed.sessions : []
            delegate: PlasmaComponents.ItemDelegate {
                required property var modelData
                text: modelData.title
                Layout.fillWidth: true
                onClicked: Qt.openUrlExternally(modelData.url)
            }
        }
    }
}
