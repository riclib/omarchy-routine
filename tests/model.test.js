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
  "platformIcon, within_depth, safeUrl, safeId, MAX_ANSWER, MAX_LINE, MAX_TITLE})")(Model)

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
    events: new Array(90).fill({ title: "e", at: "09:00" }),
    tasks: [{ id: "task:aB0_cD1eF2gH3iJ4kL5m", title: "real", done: false },
            { id: "javascript:alert(1)", title: "hostile id", done: false }],
  }))
  assert.equal(d.ok, true)
  assert.equal(d.next.join, "https://teams.microsoft.com/meet/1")
  assert.ok(d.events.length <= 20, "event list is capped")
  assert.equal(d.tasks[0].id, "task:aB0_cD1eF2gH3iJ4kL5m")
  assert.equal(d.tasks[1].id, "", "an id that fails its shape check is blanked")
})

test("the countdown says what a person would say", () => {
  assert.equal(Model.gap(0), "now")
  assert.equal(Model.gap(21), "21m")
  assert.equal(Model.gap(60), "1h")
  assert.equal(Model.gap(125), "2h5m")
  assert.equal(Model.gapSentence(31), "in 31 min")
})
