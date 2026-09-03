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
  readonly property string askModel: {
    var v = shell?.pluginSetting?.(manifest?.id || "riclib.routine", "askModel", "")
    return String(v || "")
  }

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

  // Two things you can do with one box. Capture is the fast path and never
  // costs a model call; ask is deliberate, and Tab is how you say which.
  property string mode: "log"                 // "log" | "ask"
  readonly property bool asking: mode === "ask"
  property bool thinking: false

  // One conversation per opening. The transcript is rtn's, keyed by an id
  // minted here; what this holds is only what is drawn — the question and
  // the answer, as text. The session ends when the overlay closes.
  property string session: ""
  property var thread: []                     // [{ q, a, failed }]
  readonly property var visibleThread: root.thread.slice(-3)

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
    root.showPast = false
    root.thread = []
    root.session = Date.now().toString(36) + "-" + Math.random().toString(36).slice(2, 10)
    root.mode = "log"
    root.pending = ({})
    root.refresh()
    root.opened = true
  }

  function close() {
    root.opened = false
    // Nothing to forget unless something was asked.
    if (root.thread.length > 0)
      root.enqueue([root.rtnBin, "ask", "--session", root.session, "--end"])
    root.thread = []
  }
  function toggle() { root.opened ? root.close() : root.open("{}") }

  function refresh() {
    if (dash.running) return
    root.nowMs = Date.now()
    dash.running = false
    dash.running = true
  }

  function isDone(task) {
    return root.pending[task.id] !== undefined ? root.pending[task.id] : task.done
  }

  readonly property int openCount: {
    var n = 0
    for (var i = 0; i < cache.tasks.length; i++) if (!isDone(cache.tasks[i])) n++
    for (var j = 0; j < cache.agenda.length; j++) {
      var e = cache.agenda[j]
      if (e.kind === "block" && !isDone({ id: e.task, done: e.done })) n++
    }
    return n
  }

  // The agenda splits at the clock: what is over collapses to one line so the
  // card does not grow all afternoon, and what is still to come stays.
  property bool showPast: false
  readonly property var pastAgenda: cache.agenda.filter(function(e) { return Model.isPast(e.end, root.nowMs) })
  readonly property var comingAgenda: cache.agenda.filter(function(e) { return !Model.isPast(e.end, root.nowMs) })
  readonly property var shownAgenda: root.showPast ? cache.agenda : root.comingAgenda

  Process {
    id: dash
    command: [root.rtnBin, "dashboard", "--json"]
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var parsed = Model.parseDashboard(text)
        if (parsed.ok) {
          // Drop only the optimistic ticks the server has caught up with.
          // A journal task's `done` comes from the note's checkbox, which
          // trails the task by several seconds through the app's own sync,
          // so clearing them all would flip a box back that is merely not
          // synced yet — and it would do it while you watched.
          var unconfirmed = ({})
          for (var i = 0; i < parsed.tasks.length; i++) {
            var t = parsed.tasks[i]
            if (root.pending[t.id] !== undefined && root.pending[t.id] !== t.done)
              unconfirmed[t.id] = root.pending[t.id]
          }
          root.pending = unconfirmed
        }
        root.cache = parsed
      }
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = Model.text(text, Model.MAX_LINE)
        if (message !== "") root.cache = Model.failed(message.split("\n")[0])
      }
    }
  }

  // One process, several clicks. Assigning `command` while it is running loses
  // the new one, and ticking three boxes quickly is an ordinary thing to do —
  // so they queue and drain rather than racing.
  property var queue: []

  Process {
    id: actionProc
    stdout: StdioCollector { waitForEnd: true }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = Model.text(text, Model.MAX_LINE)
        if (message !== "") root.flash = message.split("\n")[0]
      }
    }
    onRunningChanged: if (!running) root.drain()
  }

  function enqueue(command) {
    var pendingCommands = root.queue.slice()
    pendingCommands.push(command)
    root.queue = pendingCommands
    root.drain()
  }

  function drain() {
    if (actionProc.running || root.queue.length === 0) return
    var rest = root.queue.slice()
    var next = rest.shift()
    root.queue = rest
    actionProc.command = next
    actionProc.running = true
  }

  Process {
    id: logProc
    stdout: StdioCollector { waitForEnd: true }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = Model.text(text, Model.MAX_LINE)
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

  // Enter appends to today's log. Shift+Enter files a task in the Inbox --
  // unplanned and unparented, which is where a thought goes before it is a
  // plan. The journal-bound variant (`rtn log --task`) is the other one, and
  // this box deliberately does not offer it: one box, two obvious outcomes.
  function submit(asTask) {
    var body = root.draft.trim()
    if (body === "" || root.busy) return
    root.busy = true
    root.flash = ""
    // Passed as arguments, never through a shell — the text is whatever was
    // typed, and quoting is not a security model.
    logProc.command = asTask
      ? [root.rtnBin, "add", body]
      : [root.rtnBin, "log", body]
    logProc.running = false
    logProc.running = true
  }

  function ask() {
    var question = root.draft.trim()
    if (question === "" || root.thinking) return
    root.thinking = true
    root.flash = ""
    var argv = [root.rtnBin, "ask", "--session", root.session, question]
    if (root.askModel !== "") argv.push("--model", root.askModel)
    askProc.question = question
    askProc.command = argv
    askProc.running = false
    askProc.running = true
  }

  // Tab out and back keeps the thread: the conversation is the opening,
  // not the mode.
  function setMode(next) {
    root.mode = next
    root.flash = ""
  }

  function reply(question, answer, failed) {
    var next = root.thread.slice()
    next.push({ q: question, a: answer, failed: failed })
    root.thread = next
  }

  Process {
    id: askProc
    property string question: ""
    property string answer: ""
    stdout: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        // Clamped here, at the boundary, rather than at the Text that draws
        // it: the sink is not the only caller, and a second one added later
        // would inherit the gap rather than the guard.
        askProc.answer = Model.text(text, Model.MAX_ANSWER)
      }
    }
    stderr: StdioCollector {
      waitForEnd: true
      onStreamFinished: {
        var message = Model.text(text, Model.MAX_LINE)
        if (message !== "") root.flash = message.split("\n")[0]
      }
    }
    onExited: function(code) {
      root.thinking = false
      if (code === 0) {
        root.reply(askProc.question, askProc.answer, false)
        root.draft = ""
        // A question may well have changed something.
        root.refresh()
      } else {
        root.reply(askProc.question, askProc.answer !== "" ? askProc.answer
          : "That did not work. `rtn ask` needs a model and a key in ~/.config/rtn/ask.yaml; `rtn doctor` says what it found.", true)
      }
      askProc.answer = ""
    }
  }

  function toggleTask(task) {
    // Model.safeId() blanks anything that is not shaped like a Routine id, so
    // an empty one here means "did not pass", not "absent".
    if (task.id === "") return
    var next = !isDone(task)
    // A **copy**, not the same object mutated. Assigning the identical
    // reference back is not a change as far as QML is concerned, so no
    // binding re-evaluates and the tick goes to Routine without ever
    // reaching the pixel you clicked.
    var updated = ({})
    for (var key in root.pending) updated[key] = root.pending[key]
    updated[task.id] = next
    root.pending = updated
    root.enqueue([root.rtnBin, "task", next ? "done" : "open", task.id])
  }

  function join() {
    if (cache.next && cache.next.join !== "")
      root.enqueue(["xdg-open", cache.next.join])
  }

  function joinItem(item) {
    if (item && item.join !== "") root.enqueue(["xdg-open", item.join])
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
              text: root.cache.next ? (root.cache.next.kind === "block" ? "NEXT BLOCK" : "NEXT") : ""
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

        // ---- the agenda: meetings and blocks, in order ----------------------
        Row {
          width: parent.width
          spacing: Style.spacing.xs
          visible: root.cache.ok && root.cache.agenda.length > 0
          Text {
            text: "Scheduled"
            textFormat: Text.PlainText
            color: root.foreground
            font.pixelSize: Style.font.subtitle
            font.weight: Font.DemiBold
          }
          Text {
            text: root.pastAgenda.length === 0 ? ""
              : (root.showPast ? "hide earlier" : root.pastAgenda.length + " earlier")
            textFormat: Text.PlainText
            color: pastArea.containsMouse ? root.foreground : root.muted
            font.pixelSize: Style.font.body
            anchors.verticalCenter: parent.verticalCenter
            MouseArea {
              id: pastArea
              anchors.fill: parent
              hoverEnabled: true
              cursorShape: Qt.PointingHandCursor
              onClicked: root.showPast = !root.showPast
            }
          }
        }

        Column {
          width: parent.width
          spacing: Style.spacing.hairline
          visible: root.cache.ok && root.cache.agenda.length > 0

          add: Transition {
            NumberAnimation { properties: "opacity"; from: 0; to: 1; duration: 260 }
            NumberAnimation { properties: "y"; duration: 220; easing.type: Easing.OutCubic }
          }
          move: Transition {
            NumberAnimation { properties: "y"; duration: 220; easing.type: Easing.OutCubic }
          }

          Repeater {
            model: root.shownAgenda
            delegate: AgendaRow {
              required property var modelData
              width: contentColumn.width
              item: modelData
              done: modelData.kind === "block" && root.isDone({ id: modelData.task, done: modelData.done })
              past: Model.isPast(modelData.end, root.nowMs)
              foreground: root.foreground
              muted: root.muted
              accent: root.accent
              onToggled: root.toggleTask({ id: modelData.task, done: modelData.done })
              onJoined: { root.joinItem(modelData); root.close() }
            }
          }
        }

        // ---- anytime today: the tasks with no time on them ---------------
        Row {
          width: parent.width
          spacing: Style.spacing.xs
          Text {
            text: "Anytime"
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
          visible: text !== ""
          text: root.cache.ok && root.cache.tasks.length === 0
            ? (root.cache.agenda.length > 0 ? "Nothing without a time." : "Nothing left for today.")
            : (root.cache.ok ? "" : (root.cache.error + "  —  try: rtn doctor"))
          textFormat: Text.PlainText
          color: root.muted
          font.pixelSize: Style.font.body
          wrapMode: Text.WordWrap
        }

        Column {
          width: parent.width
          spacing: Style.spacing.hairline

          // A task the agent just made should arrive rather than appear, so
          // the change is something you watched happen.
          add: Transition {
            NumberAnimation { properties: "opacity"; from: 0; to: 1; duration: 260 }
            NumberAnimation { properties: "y"; duration: 220; easing.type: Easing.OutCubic }
          }
          move: Transition {
            NumberAnimation { properties: "y"; duration: 220; easing.type: Easing.OutCubic }
          }

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
        // Below the day, above the box: you asked from what you can see, and the
        // answer lands next to it rather than in place of it.
        // The last few exchanges, oldest dimmed, so a follow-up is read in
        // the light of what it follows. rtn remembers more than is drawn.
        Rectangle {
          width: parent.width
          height: threadColumn.implicitHeight + Style.space(14)
          radius: Style.space(5)
          color: Qt.rgba(root.accent.r, root.accent.g, root.accent.b, 0.07)
          visible: root.asking && (root.thinking || root.thread.length > 0)
          opacity: visible ? 1 : 0
          Behavior on opacity { NumberAnimation { duration: 200 } }

          Column {
            id: threadColumn
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            anchors.margins: Style.space(7)
            spacing: Style.spacing.sm

            Repeater {
              model: root.visibleThread
              delegate: Column {
                required property var modelData
                required property int index
                width: threadColumn.width
                spacing: Style.spacing.xxs
                opacity: index === root.visibleThread.length - 1 && !root.thinking ? 1.0 : 0.6

                Text {
                  width: parent.width
                  text: "› " + modelData.q
                  textFormat: Text.PlainText
                  color: root.muted
                  font.pixelSize: Style.font.bodySmall
                  wrapMode: Text.WordWrap
                  maximumLineCount: 2
                  elide: Text.ElideRight
                }
                Text {
                  width: parent.width
                  text: modelData.a
                  // A model's words are still a server's words.
                  textFormat: Text.PlainText
                  color: modelData.failed ? root.muted : root.foreground
                  font.pixelSize: Style.font.body
                  wrapMode: Text.WordWrap
                }
              }
            }

            Text {
              visible: root.thinking
              text: "thinking…"
              textFormat: Text.PlainText
              color: root.muted
              font.pixelSize: Style.font.body
              opacity: 0.7
            }
          }
        }

        TextField {
          id: input
          width: parent.width
          foreground: root.foreground
          accent: root.accent
          placeholderText: root.asking
            ? "ask about your Routine   ·   Tab to go back"
            : "log to today   ·   Shift+Enter files a task   ·   Tab to ask"
          enabled: !root.busy && !root.thinking
          text: root.draft
          onTextChanged: root.draft = text
          focus: root.opened

          Keys.onPressed: function(event) {
            var isEnter = event.key === Qt.Key_Return || event.key === Qt.Key_Enter
            if (event.key === Qt.Key_Escape) {
              // Esc walks back one step: out of asking first, then out of here.
              if (root.asking) root.setMode("log")
              else root.close()
              event.accepted = true
            } else if (event.key === Qt.Key_Tab || event.key === Qt.Key_Backtab) {
              root.setMode(root.asking ? "log" : "ask")
              event.accepted = true
            } else if (isEnter && root.asking) {
              root.ask()
              event.accepted = true
            } else if (isEnter) {
              root.submit(event.modifiers & Qt.ShiftModifier)
              event.accepted = true
            }
          }
        }

        Text {
          width: parent.width
          text: {
            if (root.flash !== "") return root.flash
            if (root.asking) return "Enter to ask   ·   Tab back to logging   ·   Esc closes"
            return "Enter logs   ·   Shift+Enter files a task   ·   Tab to ask"
          }
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
