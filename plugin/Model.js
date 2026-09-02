// Parsing and arithmetic for the Routine widget. Plain JS on purpose: it has
// no QML in it, so it can be reasoned about — and tested — without a shell.
//
// Everything here treats `rtn`'s output as a shape it does not control. The CLI
// already clamps and drops what a widget must not render, but a widget that
// assumes well-formed input is one malformed reply away from an empty bar, so
// every field is defaulted rather than trusted.

.pragma library

var MAX_EVENTS = 20
var MAX_TASKS = 40
var MAX_TITLE = 160

function str(value, fallback) {
  if (typeof value !== "string") return fallback || ""
  // A title can arrive with newlines in it, straight from the API.
  var flat = value.replace(/\s+/g, " ").trim()
  return flat.length > MAX_TITLE ? flat.slice(0, MAX_TITLE) : flat
}

function num(value, fallback) {
  var n = Number(value)
  return isFinite(n) ? n : (fallback || 0)
}

function empty() {
  return { ok: false, title: "", next: null, events: [], tasks: [], open: 0, error: "" }
}

function failed(message) {
  var blank = empty()
  blank.error = str(message, "could not read Routine")
  return blank
}

function parseEvent(raw) {
  if (!raw || typeof raw !== "object") return null
  return {
    id: str(raw.id),
    title: str(raw.title),
    at: str(raw.at),
    start: str(raw.start),
    minutes: num(raw.minutes, 0),
    platform: str(raw.platform),
    // A join link only exists when the CLI matched an allowlisted host. It is
    // never assembled here, and never taken from anywhere else.
    join: str(raw.join)
  }
}

function parseDashboard(text) {
  var doc
  try {
    doc = JSON.parse(String(text || ""))
  } catch (e) {
    return failed("Routine returned something unreadable")
  }
  if (!doc || typeof doc !== "object") return failed("Routine returned nothing")

  var out = empty()
  out.ok = true
  out.title = str(doc.title)
  out.next = parseEvent(doc.next)
  out.open = num(doc.open, 0)

  var events = Array.isArray(doc.events) ? doc.events.slice(0, MAX_EVENTS) : []
  for (var i = 0; i < events.length; i++) {
    var e = parseEvent(events[i])
    if (e) out.events.push(e)
  }

  var tasks = Array.isArray(doc.tasks) ? doc.tasks.slice(0, MAX_TASKS) : []
  for (var j = 0; j < tasks.length; j++) {
    var t = tasks[j]
    if (!t || typeof t !== "object") continue
    out.tasks.push({
      id: str(t.id),
      title: str(t.title),
      done: t.done === true,
      source: str(t.source)
    })
  }
  return out
}

// Minutes remaining, recomputed locally between polls. The data costs a call;
// the ticking costs nothing, so the countdown stays live without asking again.
function minutesLeft(startIso, nowMs) {
  if (!startIso) return null
  var start = Date.parse(startIso)
  if (!isFinite(start)) return null
  return Math.round((start - nowMs) / 60000)
}

// How long until it, said the way a person would. Short enough for a bar.
function gap(minutes) {
  if (minutes === null || minutes === undefined) return ""
  if (minutes <= 0) return "now"
  if (minutes < 60) return minutes + "m"
  var hours = Math.floor(minutes / 60)
  var rest = minutes % 60
  if (hours >= 10 || rest === 0) return hours + "h"
  return hours + "h" + rest + "m"
}

// The same thing in a sentence, for a tooltip that has room.
function gapSentence(minutes) {
  if (minutes === null || minutes === undefined) return ""
  if (minutes <= 0) return "starting now"
  if (minutes < 60) return "in " + minutes + " min"
  var hours = Math.floor(minutes / 60)
  var rest = minutes % 60
  if (rest === 0) return "in " + hours + (hours === 1 ? " hour" : " hours")
  return "in " + hours + "h " + rest + "m"
}

// A glyph per meeting platform, chosen from a closed set. The platform name
// comes from the server, so it selects among icons we picked rather than
// becoming one.
function platformIcon(platform) {
  switch (platform) {
    case "teams": return "󰊻"
    case "zoom": return "󰡉"
    case "meet": return "󰕧"
    case "webex": return "󰕧"
    case "jitsi": return "󰕧"
    case "whereby": return "󰕧"
    default: return ""
  }
}
