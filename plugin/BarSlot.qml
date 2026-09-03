import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import "Model.js" as Model

// The bar slot for Routine: how long until the next thing, permanently, which
// is the one thing Routine's own dashboard cannot do because you have to open
// it — and opening it takes anywhere between no time and a minute.
//
// This widget never speaks MCP. `rtn dashboard --json` holds the token, bounds
// every string and decides what is safe to render; here we only draw it. That
// split is deliberate: a credential does not belong in the shell's process
// state, and neither does the job of parsing a meeting invite.
BarWidget {
  id: root
  moduleName: "riclib.routine"

  // nf-md-calendar_clock, written as an escape rather than the literal
  // character: a raw private-use-area codepoint does not survive every editor
  // that touches this file, and when it is dropped the widget renders as a
  // bare number.
  readonly property string icon: "󰃰"

  readonly property string rtnBin: String(setting("rtnBin", "rtn") || "rtn")
  readonly property int refreshSeconds: Math.max(15, Number(setting("refreshSeconds", 60)) || 60)
  readonly property int urgentMinutes: Math.max(0, Number(setting("urgentMinutes", 5)) || 0)

  // Named `cache`, not `data`: `data` is Item's default property — the list of
  // child objects — so declaring it makes every read return children instead.
  property var cache: Model.empty()
  property double nowMs: Date.now()

  // The countdown is local arithmetic on one fetched timestamp. Polling for it
  // would be asking the same question every second and getting the same answer.
  readonly property var minutesLeft: root.cache.next
    ? Model.minutesLeft(root.cache.next.start, root.nowMs) : null
  readonly property bool imminent: root.urgentMinutes > 0
    && root.minutesLeft !== null
    && root.minutesLeft <= root.urgentMinutes
    && root.minutesLeft > -5

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // Every instance polls for itself rather than electing a leader. A call is
  // ~10ms against local memory, so five monitors cost nothing worth saving —
  // and the leader pattern is exactly what leaves nixfred.blip opening its
  // panel on the wrong screen.
  Process {
    id: dash
    command: [root.rtnBin, "dashboard", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: root.cache = Model.parseDashboard(text)
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = Model.text(text, Model.MAX_LINE)
        if (message !== "") root.cache = Model.failed(message.split("\n")[0])
      }
    }
    // A missing binary is the ordinary first-run failure, and it has to say so
    // rather than rendering an empty bar with no explanation.
    onExited: function(code) {
      if (code !== 0 && root.cache.ok)
        root.cache = Model.failed("rtn exited " + code)
    }
  }

  function refresh() {
    if (dash.running) return
    // A one-shot Process needs the reset, or the second run is a no-op.
    dash.running = false
    dash.running = true
  }

  Timer {
    interval: root.refreshSeconds * 1000
    running: true
    repeat: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  // Ten seconds is finer than the label can show below an hour and coarser
  // than a redraw anyone notices.
  Timer {
    interval: 10000
    running: true
    repeat: true
    onTriggered: root.nowMs = Date.now()
  }

  readonly property string gapText: root.minutesLeft === null ? "" : Model.gap(root.minutesLeft)

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    labelVisible: !root.vertical
    active: root.imminent
    text: {
      if (root.vertical) return root.icon
      if (!root.cache.ok) return root.icon
      if (root.gapText !== "") return root.icon + "  " + root.gapText
      // Nothing left today is worth saying plainly rather than with a blank.
      return root.cache.open > 0 ? root.icon + "  " + root.cache.open : root.icon
    }

    tooltipText: {
      if (!root.cache.ok)
        return "Routine — " + (root.cache.error || "not answering")
          + "\nIs Routine running with its MCP server on? Try: rtn doctor"
      var lines = []
      if (root.cache.next) {
        var platform = Model.platformIcon(root.cache.next.platform)
        lines.push(root.cache.next.at + "  " + root.cache.next.title
          + (platform !== "" ? "  " + platform : ""))
        lines.push(Model.gapSentence(root.minutesLeft))
      } else {
        lines.push("Nothing else on the calendar today")
      }
      lines.push(root.cache.open + (root.cache.open === 1 ? " task open" : " tasks open"))
      lines.push("click to open Routine"
        + (root.cache.next && root.cache.next.join !== "" ? "   ·   middle click to join" : ""))
      return lines.join("\n")
    }

    onPressed: function(pressedButton) {
      if (!root.bar) return
      if (pressedButton === Qt.MiddleButton) {
        // Only a link rtn matched against its host allowlist ever gets here,
        // and it is passed as one argument rather than through a shell.
        if (root.cache.next && root.cache.next.join !== "") {
          joinProcess.command = ["xdg-open", root.cache.next.join]
          joinProcess.running = false
          joinProcess.running = true
        }
      } else if (pressedButton === Qt.RightButton) {
        root.refresh()
      } else {
        root.bar.run("routine-focus")
      }
    }
  }

  Process { id: joinProcess }

  // An IPC target routes to exactly one handler while this widget is live once
  // per monitor, so a refresh broadcasts — a refresh is not a place.
  IpcHandler {
    target: "riclib.routine.bar"
    function refresh(): void { root.broadcast("refresh") }
  }
}
