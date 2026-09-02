import QtQuick
import qs.Commons
import qs.Ui

// One line of today. The box is the whole point: ticking it here is the same
// act as ticking it in the daily note, because the block and the task are two
// views of one thing.
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
    border.color: row.done ? row.accent : row.muted
    border.width: 1

    Text {
      anchors.centerIn: parent
      text: "✓"
      textFormat: Text.PlainText
      visible: row.done
      color: row.foreground
      font.pixelSize: Style.font.bodySmall
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
  }
}
