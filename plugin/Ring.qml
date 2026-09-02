import QtQuick
import qs.Commons
import qs.Ui

// The countdown, drawn. The arc is not decoration: it fills as the hour before
// a meeting runs out, so the shape says "soon" before the number is read.
//
// It ticks on local arithmetic against one fetched timestamp — the data costs
// a call, the clock face costs nothing.
Item {
  id: ring

  // null when there is nothing left today, which draws an empty track.
  property var minutes: null
  property string label: ""
  property string caption: ""
  property color accent: "#6aa9ff"
  property color track: "#222"
  property color foreground: "#fff"
  property color muted: "#999"

  // An hour is the window worth showing. Beyond it the arc would barely move
  // and the number is doing the work anyway.
  readonly property int window: 60
  readonly property real fraction: {
    if (minutes === null || minutes === undefined) return 0
    if (minutes <= 0) return 1
    if (minutes >= window) return 0
    return (window - minutes) / window
  }

  implicitWidth: Style.space(116)
  implicitHeight: Style.space(116)
  width: implicitWidth
  height: implicitHeight

  onFractionChanged: arc.requestPaint()
  onAccentChanged: arc.requestPaint()
  onTrackChanged: arc.requestPaint()

  Canvas {
    id: arc
    anchors.fill: parent
    onPaint: {
      var ctx = getContext("2d")
      ctx.reset()
      var thickness = Math.max(3, ring.width * 0.07)
      var radius = (Math.min(ring.width, ring.height) - thickness) / 2
      var cx = ring.width / 2
      var cy = ring.height / 2
      var start = -Math.PI / 2

      ctx.lineWidth = thickness
      ctx.lineCap = "round"

      ctx.beginPath()
      ctx.strokeStyle = ring.track
      ctx.arc(cx, cy, radius, 0, Math.PI * 2)
      ctx.stroke()

      if (ring.fraction > 0) {
        ctx.beginPath()
        ctx.strokeStyle = ring.accent
        ctx.arc(cx, cy, radius, start, start + Math.PI * 2 * ring.fraction)
        ctx.stroke()
      }
    }
  }

  Column {
    anchors.centerIn: parent
    spacing: 0

    Text {
      anchors.horizontalCenter: parent.horizontalCenter
      text: ring.label
      textFormat: Text.PlainText
      color: ring.foreground
      font.pixelSize: Style.font.display
      font.weight: Font.DemiBold
    }
    Text {
      anchors.horizontalCenter: parent.horizontalCenter
      text: ring.caption
      textFormat: Text.PlainText
      color: ring.muted
      font.pixelSize: Style.font.caption
    }
  }
}
