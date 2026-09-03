// The parsing and bounds, tested without a shell.
//
// Model.js is QML's flavour of JS — `.pragma library` is a QML directive and
// not valid JavaScript — so the file is read, the directive stripped, and the
// rest evaluated. That keeps one copy of the code rather than a testable
// duplicate that drifts.
const { test } = require("node:test")
const assert = require("node:assert")
const fs = require("node:fs")
const path = require("node:path")

const src = fs
  .readFileSync(path.join(__dirname, "..", "plugin", "Model.js"), "utf8")
  .replace(/^\s*\.pragma\s+library\s*$/m, "")
const Model = {}
new Function("exports", src + "\n;Object.assign(exports, {" +
  "str, text, num, empty, failed, parseDashboard, minutesLeft, gap, gapSentence," +
  "platformIcon, within_depth, safeUrl, safeId, isPast, length, MAX_ANSWER, MAX_LINE, MAX_TITLE})")(Model)

test("a title arrives collapsed and clamped", () => {
  assert.equal(Model.str("\n Automate  the\n thing \n"), "Automate the thing")
  assert.equal(Model.str("x".repeat(400)).length, Model.MAX_TITLE)
})

test("free text is clamped and marked, not silently cut", () => {
  const long = Model.text("y".repeat(9000), Model.MAX_ANSWER)
  assert.equal(long.length, Model.MAX_ANSWER + 1)
  assert.ok(long.endsWith("…"))
  assert.equal(Model.text(undefined, 10), "")
})

test("only an https URL survives to reach xdg-open", () => {
  assert.equal(
    Model.safeUrl("https://teams.microsoft.com/meet/1?p=x"),
    "https://teams.microsoft.com/meet/1?p=x")
  // Anything that is not plainly a URL is dropped rather than repaired.
  for (const bad of [
    "http://insecure.example/x",
    "file:///etc/passwd",
    "javascript:alert(1)",
    "https://ok.example/x; rm -rf /",
    'https://ok.example/"quoted"',
    "https://" + "a".repeat(600),
    "",
  ]) assert.equal(Model.safeUrl(bad), "", `should have refused: ${bad}`)
})

test("only a Routine-shaped id is echoed back as an argument", () => {
  assert.equal(Model.safeId("task:b0bvpSXg0S_DHAvVxhu2W"), "task:b0bvpSXg0S_DHAvVxhu2W")
  assert.equal(Model.safeId("object:ws:tbl:row"), "object:ws:tbl:row")
  for (const bad of ["--help", "task:", "; reboot", "task:a b", "TASK:x", "x".repeat(400)])
    assert.equal(Model.safeId(bad), "", `should have refused: ${bad}`)
})

test("nesting is refused before the parser sees it", () => {
  assert.ok(Model.within_depth('{"a":[{"b":1}]}', 12))
  assert.ok(!Model.within_depth("[".repeat(400), 12))
  // Brackets inside a string are content, not structure.
  assert.ok(Model.within_depth('{"a":"[[[[[[[[[[[[[[[["}', 12))
  assert.ok(Model.within_depth('{"a":"\\"[[[[[[[[[[[[[[[["}', 12))
})

test("an oversized or hostile reply degrades instead of landing", () => {
  const huge = Model.parseDashboard('{"x":"' + "z".repeat(600000) + '"}')
  assert.equal(huge.ok, false)
  assert.match(huge.error, /more than this can hold/)

  const deep = Model.parseDashboard("[".repeat(300) + "]".repeat(300))
  assert.equal(deep.ok, false)
  assert.match(deep.error, /deeply nested/)

  assert.equal(Model.parseDashboard("not json").ok, false)
  assert.equal(Model.parseDashboard("").ok, false)
})

test("a good payload keeps its shape and drops what fails a check", () => {
  const d = Model.parseDashboard(JSON.stringify({
    title: "September 3, 2026",
    open: 1,
    next: { title: "Catch up", at: "16:00", start: "2026-09-03T16:00:00+03:00",
            platform: "teams", join: "https://teams.microsoft.com/meet/1" },
    agenda: new Array(90).fill({ title: "e", at: "09:00" }),
    tasks: [{ id: "task:aB0_cD1eF2gH3iJ4kL5m", title: "real", done: false },
            { id: "javascript:alert(1)", title: "hostile id", done: false }],
  }))
  assert.equal(d.ok, true)
  assert.equal(d.next.join, "https://teams.microsoft.com/meet/1")
  assert.ok(d.agenda.length <= 20, "the agenda is capped")
  assert.equal(d.tasks[0].id, "task:aB0_cD1eF2gH3iJ4kL5m")
  assert.equal(d.tasks[1].id, "", "an id that fails its shape check is blanked")
})

test("a block carries a checked task id and a meeting carries none", () => {
  const d = Model.parseDashboard(JSON.stringify({
    title: "t", open: 2,
    agenda: [
      { kind: "block", title: "Pick up car", at: "14:00", end: "2026-09-03T15:00:00+03:00",
        length: 60, task: "task:XFl8t4rCDnQt3OBBDxvk7", done: false },
      { kind: "block", title: "hostile", at: "15:00", task: "--help; reboot", done: true },
      { kind: "meeting", title: "Catch up", at: "16:00", join: "https://teams.microsoft.com/meet/9" },
      { kind: "meeting", title: "bad link", at: "17:00", join: "file:///etc/passwd" },
      { title: "no kind at all", at: "18:00", task: "task:abc", done: true },
    ],
    tasks: [],
  }))
  assert.equal(d.ok, true)
  assert.equal(d.agenda[0].kind, "block")
  assert.equal(d.agenda[0].task, "task:XFl8t4rCDnQt3OBBDxvk7")
  assert.equal(d.agenda[0].length, 60)
  assert.equal(d.agenda[1].task, "", "a block whose task fails the shape check cannot be ticked")
  assert.equal(d.agenda[1].done, true)
  // The per-row Join goes through the same gate as the NEXT card's.
  assert.equal(d.agenda[2].join, "https://teams.microsoft.com/meet/9")
  assert.equal(d.agenda[3].join, "")
  // An unknown kind is a meeting: it gets no box, and its task is ignored.
  assert.equal(d.agenda[4].kind, "meeting")
  assert.equal(d.agenda[4].task, "")
  assert.equal(d.agenda[4].done, false)
})

test("what is over is told by the clock, and a length is said short", () => {
  const noon = Date.parse("2026-09-03T12:00:00+03:00")
  assert.ok(Model.isPast("2026-09-03T11:30:00+03:00", noon))
  assert.ok(!Model.isPast("2026-09-03T12:30:00+03:00", noon))
  assert.ok(!Model.isPast("", noon), "no end is not past")
  assert.ok(!Model.isPast("garbage", noon))
  assert.equal(Model.length(30), "30m")
  assert.equal(Model.length(90), "1h30m")
  assert.equal(Model.length(0), "")
})

test("the countdown says what a person would say", () => {
  assert.equal(Model.gap(0), "now")
  assert.equal(Model.gap(21), "21m")
  assert.equal(Model.gap(60), "1h")
  assert.equal(Model.gap(125), "2h5m")
  assert.equal(Model.gapSentence(31), "in 31 min")
})
