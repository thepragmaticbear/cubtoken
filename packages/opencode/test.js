// Self-check for the OpenCode adapter. No framework: `node test.js`.
// Needs a real ctk — set CUBTOKEN_CTK_BIN, or have one on PATH.
// `cargo test --test opencode` runs this with the freshly built binary.

import assert from "node:assert/strict"
import { mkdtempSync } from "node:fs"
import { tmpdir } from "node:os"
import { join } from "node:path"

import { CubtokenPlugin, parseRead, rebuildRead } from "./index.js"

const cwd = mkdtempSync(join(tmpdir(), "cubtoken-opencode-"))

/// Mirrors packages/opencode/src/tool/read.ts's output assembly.
function openCodeRead(filePath, text) {
  const lines = text.split("\n")
  return (
    `<path>${filePath}</path>\n<type>file</type>\n<content>\n` +
    lines.map((line, i) => `${i + 1}: ${line}`).join("\n") +
    `\n\n(End of file - total ${lines.length} lines)` +
    `\n</content>`
  )
}

function bigRustFile() {
  let src = ""
  for (let i = 0; i < 40; i++) {
    src += `/// Does operation ${i}.\npub fn operation_${i}(store: &mut Store, id: u64) -> Option<String> {\n`
    for (let j = 0; j < 20; j++) src += `    let step_${j} = id + ${j};\n`
    src += "    None\n}\n\n"
  }
  return src
}

const hooks = await CubtokenPlugin({ directory: cwd })
const after = hooks["tool.execute.after"]

// --- round trip: parse strips OpenCode's numbering, rebuild restores the envelope
{
  const text = "fn a() {}\nfn b() {}"
  const parsed = parseRead(openCodeRead("/tmp/x.rs", text))
  assert.equal(parsed.text, text, "parseRead should recover the verbatim file text")
  assert.match(parsed.head, /<path>\/tmp\/x\.rs<\/path>/)
  assert.match(parsed.note, /End of file - total 2 lines/)
  const rebuilt = rebuildRead(parsed, "1: skeleton")
  assert.match(rebuilt, /<content>/)
  assert.match(rebuilt, /<\/content>$/)
}

// --- a large file is replaced with a skeleton
{
  const src = bigRustFile()
  const output = { output: openCodeRead("/tmp/big.rs", src), title: "big.rs", metadata: {} }
  const before = output.output
  await after({ tool: "read", sessionID: "s1", callID: "c1", args: { filePath: "/tmp/big.rs" } }, output)

  assert.notEqual(output.output, before, "large read should have been compressed")
  assert.match(output.output, /cubtoken: compressed view/)
  assert.ok(output.output.length < before.length, "compressed output should be smaller")
  assert.match(output.output, /<path>\/tmp\/big\.rs<\/path>/, "envelope should survive")
  assert.match(output.output, /<\/content>$/, "envelope should survive")
  // A signature the skeleton must keep, at its real line number.
  assert.match(output.output, /pub fn operation_0/)
  // Invariant 2: the escape hatch must name a call OpenCode actually has.
  assert.match(
    output.output,
    /run read\(filePath=\/tmp\/big\.rs/,
    "escape hatch should use OpenCode's read(filePath=…) spelling",
  )
  assert.doesNotMatch(output.output, /Read\(file_path=/)
}

// --- a small file passes through untouched
{
  const output = { output: openCodeRead("/tmp/small.rs", "fn main() {}"), title: "", metadata: {} }
  const before = output.output
  await after({ tool: "read", sessionID: "s1", callID: "c2", args: { filePath: "/tmp/small.rs" } }, output)
  assert.equal(output.output, before, "small read should pass through")
}

// --- a targeted read is never compressed (the escape hatch)
{
  const output = { output: openCodeRead("/tmp/big.rs", bigRustFile()), title: "", metadata: {} }
  const before = output.output
  await after(
    { tool: "read", sessionID: "s1", callID: "c3", args: { filePath: "/tmp/big.rs", offset: 10, limit: 20 } },
    output,
  )
  assert.equal(output.output, before, "offset/limit read should pass through")
}

// --- unrecognised output shape fails open
{
  const output = { output: "not the read envelope at all", title: "", metadata: {} }
  const before = output.output
  await after({ tool: "read", sessionID: "s1", callID: "c4", args: { filePath: "/tmp/x.rs" } }, output)
  assert.equal(output.output, before, "unknown shape should pass through")
}

// --- other tools are left alone
{
  const output = { output: "some grep rows", title: "", metadata: {} }
  const before = output.output
  await after({ tool: "grep", sessionID: "s1", callID: "c5", args: { pattern: "x" } }, output)
  assert.equal(output.output, before, "grep is out of scope for now")
}

// --- edit protection: a file edited this session is not compressed afterwards
{
  await after(
    { tool: "edit", sessionID: "s2", callID: "c6", args: { filePath: "/tmp/big.rs" } },
    { output: "ok", title: "", metadata: {} },
  )
  const output = { output: openCodeRead("/tmp/big.rs", bigRustFile()), title: "", metadata: {} }
  const before = output.output
  await after({ tool: "read", sessionID: "s2", callID: "c7", args: { filePath: "/tmp/big.rs" } }, output)
  assert.equal(output.output, before, "edited file must not be compressed")
}

console.log("ok — opencode adapter self-check passed")
