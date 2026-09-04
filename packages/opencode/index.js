// @cubtoken/opencode — runs cubtoken's compressor inside OpenCode.
//
// OpenCode's `tool.execute.after` hook hands us the tool result object that is
// then returned to the model verbatim (packages/opencode/src/session/tools.ts),
// so mutating `output.output` is what replaces what the model sees — the same
// job `updatedToolOutput` does in Claude Code.
//
// Scope: `read` compression plus `edit`/`write` ledger protection. Grep, glob
// and shell folding are deliberately left out until there is dogfooding
// evidence for them; the README's measured case is large reads.
//
// Fail open (invariant 1): every path is wrapped so a missing binary, a shape
// we don't recognise, or any thrown error leaves the original output alone.

import { spawnSync } from "node:child_process"
import { existsSync } from "node:fs"
import { homedir } from "node:os"
import { join } from "node:path"

const CONTENT_OPEN = "<content>\n"
const CONTENT_CLOSE = "\n</content>"
const CTK_TIMEOUT_MS = 5000

let cachedBin

/// PATH first, then the usual install prefixes — mirrors the Claude Code
/// plugin's wrapper script. `CUBTOKEN_CTK_BIN` overrides for tests and for
/// non-standard installs.
function candidates() {
  return [
    process.env.CUBTOKEN_CTK_BIN,
    "ctk",
    join(homedir(), ".cargo", "bin", "ctk"),
    join(homedir(), ".local", "bin", "ctk"),
    "/usr/local/bin/ctk",
  ].filter(Boolean)
}

/// Runs `ctk hook` with `payload` on stdin. Returns the parsed decision, or
/// null for "leave the output alone" — which covers a missing binary, a
/// crash, and ctk's own pass-through (empty stdout).
function runCtk(payload) {
  const input = JSON.stringify(payload)
  const tried = cachedBin ? [cachedBin] : candidates()
  for (const bin of tried) {
    if (bin.includes("/") && !existsSync(bin)) continue
    const result = spawnSync(bin, ["hook"], {
      input,
      encoding: "utf8",
      timeout: CTK_TIMEOUT_MS,
      // Makes the compressed view's escape hatch name `read(filePath=…)`
      // rather than Claude Code's `Read(file_path=…)` (invariant 2).
      env: { ...process.env, CUBTOKEN_HARNESS: "opencode" },
    })
    // ENOENT on a bare name means it isn't on PATH: try the next candidate.
    if (result.error) {
      if (cachedBin === bin) cachedBin = undefined
      continue
    }
    cachedBin = bin
    if (result.status !== 0) return null
    const stdout = (result.stdout || "").trim()
    if (!stdout) return null
    try {
      return JSON.parse(stdout)
    } catch {
      return null
    }
  }
  return null
}

/// OpenCode's read output is
/// `<path>..</path>\n<type>file</type>\n<content>\n1: line\n2: line\n\n(note)\n</content>`.
/// Peel it back to the raw file text ctk expects, keeping the wrapper so the
/// compressed view can be put back in the same envelope.
export function parseRead(output, offset = 1) {
  if (typeof output !== "string") return null
  const open = output.indexOf(CONTENT_OPEN)
  if (open < 0) return null
  const bodyStart = open + CONTENT_OPEN.length
  const close = output.lastIndexOf(CONTENT_CLOSE)
  if (close <= bodyStart) return null

  let body = output.slice(bodyStart, close)
  // Trailing "(End of file - total N lines)" / "(Showing lines a-b of N...)".
  // It states a fact about the file, not about the elided view, so it is kept.
  let note = ""
  const noteAt = body.lastIndexOf("\n\n(")
  if (noteAt >= 0 && body.endsWith(")")) {
    note = body.slice(noteAt)
    body = body.slice(0, noteAt)
  }

  const lines = body.split("\n")
  const raw = []
  for (let i = 0; i < lines.length; i++) {
    const prefix = `${offset + i}: `
    // An unexpected shape means we no longer know which text is verbatim
    // (invariant 3), so bail rather than guess.
    if (!lines[i].startsWith(prefix)) return null
    raw.push(lines[i].slice(prefix.length))
  }
  return {
    head: output.slice(0, bodyStart),
    tail: output.slice(close),
    note,
    text: raw.join("\n"),
  }
}

/// Claude Code renumbers a replaced Read down the left edge, and cubtoken's
/// header tells the model to ignore that and trust the line numbers inside the
/// view. Re-adding sequential prefixes here keeps that header true in OpenCode
/// too, instead of leaving it describing something that didn't happen.
export function rebuildRead(parsed, compressed) {
  const numbered = compressed
    .split("\n")
    .map((line, i) => `${i + 1}: ${line}`)
    .join("\n")
  return parsed.head + numbered + parsed.note + parsed.tail
}

function filePathOf(args) {
  if (!args || typeof args !== "object") return null
  return args.filePath || args.file_path || null
}

export const CubtokenPlugin = async ({ directory, worktree } = {}) => {
  const cwd = worktree || directory || process.cwd()
  return {
    "tool.execute.after": async (input, output) => {
      try {
        const tool = String(input?.tool ?? "").toLowerCase()

        // Edit protection: a file the model edited this session is never
        // compressed again. Always on, same as in Claude Code.
        if (tool === "edit" || tool === "write" || tool === "patch") {
          const filePath = filePathOf(input.args)
          if (!filePath) return
          runCtk({
            tool_name: tool === "write" ? "Write" : "Edit",
            session_id: input.sessionID ?? "",
            cwd,
            tool_input: { file_path: filePath },
            tool_response: {},
          })
          return
        }

        if (tool !== "read") return
        const args = input.args ?? {}
        // Targeted reads are the escape hatch; never compress them.
        if (args.offset != null || args.limit != null) return
        const filePath = filePathOf(args)
        if (!filePath) return

        const parsed = parseRead(output?.output)
        if (!parsed) return
        const numLines = parsed.text.split("\n").length

        const decision = runCtk({
          tool_name: "Read",
          session_id: input.sessionID ?? "",
          cwd,
          tool_input: { file_path: filePath },
          tool_response: {
            type: "text",
            file: {
              filePath,
              content: parsed.text,
              numLines,
              startLine: 1,
              totalLines: numLines,
            },
          },
        })

        const compressed =
          decision?.hookSpecificOutput?.updatedToolOutput?.file?.content
        if (typeof compressed !== "string") return
        output.output = rebuildRead(parsed, compressed)
      } catch {
        // fail open: leave the original output untouched
      }
    },
  }
}

export default CubtokenPlugin
