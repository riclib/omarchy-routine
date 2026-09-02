import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import QtQuick
import qs.Commons
import qs.Ui
import "Model.js" as Model

// The dashboard Routine has and cannot reliably show you.
//
// Its own is the right shape — a countdown, the next thing, today's list, a
// box to type into — and it takes anywhere between no time and a minute to
// appear, because it is an Electron window being built. The data behind it
// answers in about ten milliseconds. So this is not a workaround for a slow
// backend; it is a fast renderer over a source that was never the problem.
//
// It never speaks MCP. `rtn` holds the token, bounds every string, and has
// already decided which link is safe to open.
Item {
  id: root

  property var shell: null
  property var manifest: null

  readonly property string pluginDir: manifest?.__sourceDir
    || (Quickshell.env("HOME") + "/.config/omarchy/plugins/riclib.routine")
  readonly property string rtnBin: {
    var configured = shell?.pluginSetting?.(manifest?.id || "riclib.routine", "rtnBin", "rtn")
    return String(configured || "rtn")
  }

  property bool opened: false
  property var cache: Model.empty()
  property double nowMs: Date.now()
  property string draft: ""
  property string flash: ""
  property bool busy: false

  // Ticked off here the moment you click, rather than after a re-read. The
  // task updates at once but the note's checkbox follows a few seconds later
  // through the app's own sync, so waiting for the truth would look broken.
  property var pending: ({})

  readonly property var minutesLeft: cache.next
    ? Model.minutesLeft(cache.next.start, root.nowMs) : null

  // Shares the [menu] surface tokens, so a theme that styles the launcher
  // styles this too.
  readonly property color background: Color.menu.background
  readonly property color foreground: Color.menu.text
  readonly property color muted: Qt.darker(foreground, 1.55)
  readonly property color accent: Color.menu.selectedBackground
  readonly property var borderSpec: Border.surfaceSpec("menu", "border", Color.menu.border,
                                                       Math.max(1, Style.space(2)))

  function open(payloadJson) {
    root.draft = ""
    root.flash = ""
    root.pending = ({})
    root.refresh()
    root.opened = true
  }

  function close() { root.opened = false }
  function toggle() { root.opened ? root.close() : root.open("{}") }

  function refresh() {
    if (dash.running) return
    root.nowMs = Date.now()
    dash.running = true
  }

  function isDone(task) {
    return root.pending[task.id] !== undefined ? root.pending[task.id] : task.done
  }

  readonly property int openCount: {
    var n = 0
    for (var i = 0; i < cache.tasks.length; i++) if (!isDone(cache.tasks[i])) n++
    return n
  }

  Process {
    id: dash
    command: [root.rtnBin, "dashboard", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var parsed = Model.parseDashboard(text)
        if (parsed.ok) root.pending = ({})     // the reply is now the truth
        root.cache = parsed
      }
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = String(text || "").trim()
        if (message !== "") root.cache = Model.failed(message.split("\n")[0])
      }
    }
  }

  Process { id: actionProc }

  Process {
    id: logProc
    stdout: StdioCollector { waitForEnd: true }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = String(text || "").trim()
        if (message !== "") root.flash = message.split("\n")[0]
      }
    }
    onExited: function(code) {
      root.busy = false
      if (code === 0) {
        root.flash = "logged"
        root.draft = ""
        root.refresh()
      }
    }
  }

  function submit(asTask) {
    var body = root.draft.trim()
    if (body === "" || root.busy) return
    root.busy = true
    root.flash = ""
    // Passed as arguments, never through a shell — the text is whatever was
    // typed, and quoting is not a security model.
    logProc.command = asTask
      ? [root.rtnBin, "log", "--task", body]
      : [root.rtnBin, "log", body]
    logProc.running = true
  }

  function toggleTask(task) {
    if (task.id === "") return
    var next = !isDone(task)
    var updated = root.pending
    updated[task.id] = next
    root.pending = updated                      // reassign so bindings notice
    actionProc.exec([root.rtnBin, "task", next ? "done" : "open", task.id])
  }

  function join() {
    if (cache.next && cache.next.join !== "")
      actionProc.exec(["xdg-open", cache.next.join])
  }

  Timer {
    interval: 10000
    running: root.opened
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  // While it is open it is the thing you are looking at, so it stays current.
  Timer {
    interval: 30000
    running: root.opened
    repeat: true
    onTriggered: root.refresh()
  }

  PanelWindow {
    id: panel
    visible: root.opened
    anchors { top: true; bottom: true; left: true; right: true }
    color: "transparent"
    WlrLayershell.namespace: "routine-dashboard"
    WlrLayershell.layer: WlrLayer.Overlay
    WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
    exclusionMode: ExclusionMode.Ignore

    Rectangle {
      anchors.fill: parent
      color: Color.menu.scrim
    }

    MouseArea {
      anchors.fill: parent
      onClicked: root.close()
    }

    readonly property int contentMargin: Style.spacing.panelPadding

    BorderSurface {
      id: card
      width: Math.min(Style.space(560), panel.width - Style.gapsOut * 2)
      height: Math.min(contentColumn.implicitHeight + panel.contentMargin * 2,
                       panel.height - Style.gapsOut * 2)
      radius: Style.cornerRadius
      anchors.centerIn: parent
      color: root.background
      borderSpec: root.borderSpec
      padding: panel.contentMargin

      // Swallow clicks so the scrim's dismiss does not fire through the card.
      MouseArea { anchors.fill: parent }

      Column {
        id: contentColumn
        anchors.fill: parent
        anchors.margins: panel.contentMargin
        spacing: Style.spacing.md

        // ---- the day ------------------------------------------------------
        Text {
          text: root.cache.title !== "" ? root.cache.title : "Routine"
          textFormat: Text.PlainText
          color: root.foreground
          font.pixelSize: Style.font.title
          font.weight: Font.DemiBold
        }

        // ---- what is next, and how long there is --------------------------
        Item {
          width: parent.width
          height: Math.max(ring.height, nextBlock.implicitHeight)
          visible: root.cache.ok

          Ring {
            id: ring
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            minutes: root.minutesLeft
            label: root.minutesLeft === null ? "clear" : Model.gap(root.minutesLeft)
            caption: root.minutesLeft === null ? "for today" : "until next"
            accent: root.cache.next && root.minutesLeft !== null && root.minutesLeft <= 5
              ? Color.urgent : root.accent
            track: Qt.darker(root.background, 1.25)
            foreground: root.foreground
            muted: root.muted
          }

          Column {
            id: nextBlock
            anchors.left: ring.right
            anchors.leftMargin: Style.spacing.lg
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.spacing.xxs

            Text {
              text: root.cache.next ? "NEXT" : ""
              textFormat: Text.PlainText
              color: root.muted
              font.pixelSize: Style.font.caption
              font.letterSpacing: 1.5
              visible: root.cache.next !== null
            }
            Text {
              width: parent.width
              text: root.cache.next ? root.cache.next.title : "Nothing else on the calendar"
              textFormat: Text.PlainText
              color: root.foreground
              font.pixelSize: Style.font.subtitle
              wrapMode: Text.WordWrap
              maximumLineCount: 2
              elide: Text.ElideRight
            }
            Row {
              spacing: Style.spacing.xs
              visible: root.cache.next !== null
              Text {
                text: root.cache.next
                  ? root.cache.next.at + (root.cache.next.platform !== ""
                      ? "   " + Model.platformIcon(root.cache.next.platform) : "")
                  : ""
                textFormat: Text.PlainText
                color: root.muted
                font.pixelSize: Style.font.body
              }
            }
            Rectangle {
              width: joinLabel.implicitWidth + Style.space(20)
              height: joinLabel.implicitHeight + Style.space(10)
              radius: Style.space(2)
              color: joinArea.containsMouse ? root.accent : "transparent"
              border.color: root.accent
              border.width: 1
              visible: root.cache.next !== null && root.cache.next.join !== ""
              Text {
                id: joinLabel
                anchors.centerIn: parent
                text: "Join"
                textFormat: Text.PlainText
                color: root.foreground
                font.pixelSize: Style.font.body
              }
              MouseArea {
                id: joinArea
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onClicked: { root.join(); root.close() }
              }
            }
          }
        }

        // ---- today's list -------------------------------------------------
        Row {
          width: parent.width
          spacing: Style.spacing.xs
          Text {
            text: "Today"
            textFormat: Text.PlainText
            color: root.foreground
            font.pixelSize: Style.font.subtitle
            font.weight: Font.DemiBold
          }
          Text {
            text: root.openCount === 0 ? "" : root.openCount + " open"
            textFormat: Text.PlainText
            color: root.muted
            font.pixelSize: Style.font.body
            anchors.verticalCenter: parent.verticalCenter
          }
        }

        // The "well done" state is worth drawing rather than leaving a gap.
        Text {
          width: parent.width
          text: root.cache.ok && root.cache.tasks.length === 0
            ? "Nothing left for today."
            : (root.cache.ok ? "" : (root.cache.error + "  —  try: rtn doctor"))
          textFormat: Text.PlainText
          color: root.muted
          font.pixelSize: Style.font.body
          wrapMode: Text.WordWrap
          visible: text !== ""
        }

        Column {
          width: parent.width
          spacing: Style.spacing.hairline
          Repeater {
            model: root.cache.tasks
            delegate: TaskRow {
              required property var modelData
              width: contentColumn.width
              task: modelData
              done: root.isDone(modelData)
              foreground: root.foreground
              muted: root.muted
              accent: root.accent
              onToggled: root.toggleTask(modelData)
            }
          }
        }

        // ---- capture --------------------------------------------------------
        TextField {
          id: input
          width: parent.width
          foreground: root.foreground
          accent: root.accent
          placeholderText: "log to today   ·   Shift+Enter for a task"
          enabled: !root.busy
          text: root.draft
          onTextChanged: root.draft = text
          focus: root.opened

          Keys.onPressed: function(event) {
            if (event.key === Qt.Key_Escape) {
              root.close()
              event.accepted = true
            } else if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
              root.submit(event.modifiers & Qt.ShiftModifier)
              event.accepted = true
            }
          }
        }

        Text {
          width: parent.width
          text: root.flash !== "" ? root.flash : "Esc to close   ·   click a box to tick it off"
          textFormat: Text.PlainText
          color: root.muted
          font.pixelSize: Style.font.caption
          elide: Text.ElideRight
        }
      }
    }
  }

  IpcHandler {
    target: "riclib.routine"
    function open(): void { root.open("{}") }
    function close(): void { root.close() }
    function toggle(): void { root.toggle() }
  }
}
