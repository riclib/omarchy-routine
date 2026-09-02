import QtQuick
import qs.Commons
import qs.Ui

// One line of today. The box is the whole point: ticking it here is the same
// act as ticking it in the daily note, because the block and the task are two
// views of one thing.
//
// The tick animates because the write behind it is not instant — the task
// changes at once, but the note's checkbox trails it by several seconds. A box
// that fills the moment you click says "heard you" while that settles, and the
// alternative is a row that looks broken for three seconds.
Item {
  id: row

  property var task: null
  property bool done: false
  property color foreground: "#fff"
  property color muted: "#999"
  property color accent: "#6aa9ff"

  signal toggled()

  implicitHeight: Math.max(box.height, label.implicitHeight) + Style.space(8)
  height: implicitHeight

  Rectangle {
    anchors.fill: parent
    radius: Style.space(5)
    color: hover.containsMouse ? Qt.rgba(row.accent.r, row.accent.g, row.accent.b, 0.10)
                               : "transparent"
    Behavior on color { ColorAnimation { duration: 90 } }
  }

  MouseArea {
    id: hover
    anchors.fill: parent
    hoverEnabled: true
    cursorShape: Qt.PointingHandCursor
    onClicked: row.toggled()
  }

  Rectangle {
    id: box
    anchors.left: parent.left
    anchors.leftMargin: Style.space(1)
    anchors.verticalCenter: parent.verticalCenter
    width: Style.space(17)
    height: width
    radius: Style.space(4)
    color: row.done ? row.accent : "transparent"
    border.color: row.done ? row.accent : (hover.containsMouse ? row.foreground : row.muted)
    border.width: 1

    Behavior on color { ColorAnimation { duration: 140; easing.type: Easing.OutCubic } }
    Behavior on border.color { ColorAnimation { duration: 140 } }

    // A short overshoot, so ticking feels like pressing something rather than
    // like a value changing.
    scale: row.done ? 1.0 : 1.0
    SequentialAnimation on scale {
      running: false
      id: pop
      NumberAnimation { to: 1.18; duration: 90; easing.type: Easing.OutQuad }
      NumberAnimation { to: 1.0;  duration: 130; easing.type: Easing.OutBack }
    }
    Connections {
      target: row
      function onDoneChanged() { if (row.done) pop.restart() }
    }

    Text {
      anchors.centerIn: parent
      text: "✓"
      textFormat: Text.PlainText
      color: row.foreground
      font.pixelSize: Style.font.bodySmall
      opacity: row.done ? 1 : 0
      scale: row.done ? 1 : 0.4
      Behavior on opacity { NumberAnimation { duration: 120 } }
      Behavior on scale { NumberAnimation { duration: 160; easing.type: Easing.OutBack } }
    }
  }

  Text {
    id: label
    anchors.left: box.right
    anchors.leftMargin: Style.spacing.sm
    anchors.right: parent.right
    anchors.rightMargin: Style.space(1)
    anchors.verticalCenter: parent.verticalCenter
    // Every string here came from a server, so nothing is left on AutoText:
    // rich text loads resources, and what it would load is not ours to choose.
    textFormat: Text.PlainText
    text: row.task ? row.task.title : ""
    color: row.done ? row.muted : row.foreground
    font.pixelSize: Style.font.body
    font.strikeout: row.done
    elide: Text.ElideRight
    opacity: row.done ? 0.65 : 1.0

    Behavior on color { ColorAnimation { duration: 160 } }
    Behavior on opacity { NumberAnimation { duration: 160 } }
  }
}
