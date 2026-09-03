import QtQuick
import qs.Commons
import qs.Ui
import "Model.js" as Model

// One line of the day's agenda: a time, then either a meeting or a task
// blocked on the calendar. The two are drawn on the same row shape so the
// eye reads the day as one sequence, which is the point — a block is time
// you committed, and it sits in the list with the same seriousness as a
// meeting. What differs is the affordance: a block has the box, a meeting
// has Join when there is somewhere to join.
Item {
  id: row

  property var item: null
  property bool done: false
  property bool past: false
  property color foreground: "#fff"
  property color muted: "#999"
  property color accent: "#6aa9ff"

  signal toggled()
  signal joined()

  readonly property bool isBlock: item && item.kind === "block"

  implicitHeight: Math.max(Style.space(17), body.implicitHeight) + Style.space(8)
  height: implicitHeight
  opacity: past ? 0.45 : 1.0
  Behavior on opacity { NumberAnimation { duration: 200 } }

  Text {
    id: time
    anchors.left: parent.left
    anchors.leftMargin: Style.space(1)
    anchors.verticalCenter: parent.verticalCenter
    width: Style.space(44)
    text: row.item ? row.item.at : ""
    textFormat: Text.PlainText
    color: row.muted
    font.pixelSize: Style.font.bodySmall
    font.family: "monospace"
  }

  // The block half: the same row as the anytime list, box included, so a tick
  // here is the same act as a tick there.
  TaskRow {
    id: body
    anchors.left: time.right
    anchors.right: trailing.left
    anchors.rightMargin: Style.spacing.sm
    anchors.verticalCenter: parent.verticalCenter
    visible: row.isBlock
    task: row.isBlock ? ({ id: row.item.task, title: row.item.title }) : null
    done: row.done
    foreground: row.foreground
    muted: row.muted
    accent: row.accent
    onToggled: row.toggled()
  }

  // The meeting half.
  Row {
    anchors.left: time.right
    anchors.right: trailing.left
    anchors.rightMargin: Style.spacing.sm
    anchors.verticalCenter: parent.verticalCenter
    spacing: Style.spacing.sm
    visible: !row.isBlock

    Text {
      text: row.item && row.item.platform !== "" ? Model.platformIcon(row.item.platform) : "▣"
      textFormat: Text.PlainText
      color: row.muted
      font.pixelSize: Style.font.body
      width: Style.space(17)
      horizontalAlignment: Text.AlignHCenter
      anchors.verticalCenter: parent.verticalCenter
    }
    Text {
      width: parent.width - Style.space(17) - parent.spacing
      text: row.item ? row.item.title : ""
      // Every string here came from a server.
      textFormat: Text.PlainText
      color: row.foreground
      font.pixelSize: Style.font.body
      elide: Text.ElideRight
      anchors.verticalCenter: parent.verticalCenter
    }
  }

  // Length for a block; Join for a meeting that has somewhere to join, else
  // its length too.
  Item {
    id: trailing
    anchors.right: parent.right
    anchors.rightMargin: Style.space(1)
    anchors.verticalCenter: parent.verticalCenter
    width: Math.max(lengthLabel.visible ? lengthLabel.implicitWidth : 0,
                    joinButton.visible ? joinButton.width : 0)
    height: Math.max(lengthLabel.implicitHeight, joinButton.height)

    Text {
      id: lengthLabel
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      visible: !joinButton.visible
      text: row.item ? Model.length(row.item.length) : ""
      textFormat: Text.PlainText
      color: row.muted
      font.pixelSize: Style.font.caption
    }

    Rectangle {
      id: joinButton
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      width: joinLabel.implicitWidth + Style.space(16)
      height: joinLabel.implicitHeight + Style.space(6)
      radius: Style.space(2)
      visible: !row.isBlock && !row.past && row.item && row.item.join !== ""
      color: joinArea.containsMouse ? row.accent : "transparent"
      border.color: row.accent
      border.width: 1
      Text {
        id: joinLabel
        anchors.centerIn: parent
        text: "Join"
        textFormat: Text.PlainText
        color: row.foreground
        font.pixelSize: Style.font.caption
      }
      MouseArea {
        id: joinArea
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: row.joined()
      }
    }
  }
}
