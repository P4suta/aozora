// `aozora.openNotationGuide` — opens a webview pane rendering
// `media/notation-guide.md` as HTML. The doc is the canonical
// reference for everything the plugin understands; it covers the
// notation surface (注記 / ルビ / 外字 / アクセント分解 / 縦中横 /
// 帰り点) plus the plugin-side affordances (emmet, wrap menu,
// inlay, gaiji fold, diagnostic codes).
//
// We don't depend on a Markdown rendering library — the doc is
// hand-tuned and the on-disk Markdown text is rendered through a
// tiny built-in converter sufficient for this content shape
// (headings, paragraphs, lists, fenced code, inline `code`,
// tables). For richer rendering, swap in `markdown-it` later.

import * as fs from "node:fs/promises";
import * as path from "node:path";
import { commands, type ExtensionContext, ViewColumn, type WebviewPanel, window } from "vscode";

let activePanel: WebviewPanel | undefined;

export function registerNotationGuideCommand(context: ExtensionContext): void {
  context.subscriptions.push(
    commands.registerCommand("aozora.openNotationGuide", async () => {
      if (activePanel) {
        activePanel.reveal();
        return;
      }
      activePanel = window.createWebviewPanel(
        "aozoraNotationGuide",
        "Aozora 記法ガイド",
        ViewColumn.Beside,
        {
          enableScripts: false,
          retainContextWhenHidden: true,
        },
      );
      activePanel.onDidDispose(() => {
        activePanel = undefined;
      });
      const mdPath = path.join(context.extensionPath, "media", "notation-guide.md");
      let markdown = "";
      try {
        markdown = await fs.readFile(mdPath, "utf8");
      } catch (err) {
        markdown = `# 記法ガイドの読み込みに失敗\n\n\`${mdPath}\` から読み込めませんでした。\n\n${err}`;
      }
      activePanel.webview.html = renderHtml(markdown);
    }),
  );
}

// Minimal Markdown → HTML converter sufficient for our doc shape.
// Handles: ATX headings (#..######), ordered/unordered lists,
// fenced code (```), inline `code`, basic tables (| a | b |),
// paragraphs separated by blank lines, and horizontal rules.
//
// Deliberately avoids HTML-injection through user content because
// the source markdown is shipped with the extension and is not
// user-editable at runtime. Even so, we escape every text node so
// future content additions can't accidentally introduce script
// tags.
function renderHtml(md: string): string {
  const body = mdToHtml(md);
  return `<!DOCTYPE html>
<html lang="ja">
<head>
  <meta charset="UTF-8">
  <title>Aozora 記法ガイド</title>
  <style>
    :root { color-scheme: light dark; }
    body {
      font-family: var(--vscode-font-family, system-ui, sans-serif);
      font-size: var(--vscode-font-size, 14px);
      line-height: 1.6;
      padding: 1.5em 2em 4em;
      max-width: 880px;
      margin: 0 auto;
    }
    h1, h2, h3, h4 { font-weight: 600; line-height: 1.25; }
    h1 { font-size: 1.8em; border-bottom: 1px solid var(--vscode-editorWidget-border, #ddd); padding-bottom: .3em; }
    h2 { font-size: 1.4em; margin-top: 1.8em; border-bottom: 1px solid var(--vscode-editorWidget-border, #eee); padding-bottom: .2em; }
    h3 { font-size: 1.15em; margin-top: 1.4em; }
    code {
      font-family: var(--vscode-editor-font-family, ui-monospace, monospace);
      background: var(--vscode-textCodeBlock-background, rgba(127,127,127,0.1));
      padding: 1px 4px;
      border-radius: 3px;
      font-size: 0.92em;
    }
    pre {
      background: var(--vscode-textCodeBlock-background, rgba(127,127,127,0.1));
      padding: .8em 1em;
      border-radius: 4px;
      overflow-x: auto;
      line-height: 1.45;
    }
    pre code { background: transparent; padding: 0; font-size: 0.9em; }
    table {
      border-collapse: collapse;
      margin: .8em 0;
      width: 100%;
    }
    th, td {
      border: 1px solid var(--vscode-editorWidget-border, #ccc);
      padding: .35em .7em;
      text-align: left;
      vertical-align: top;
    }
    th { background: var(--vscode-list-hoverBackground, rgba(127,127,127,0.1)); font-weight: 600; }
    hr { border: none; border-top: 1px solid var(--vscode-editorWidget-border, #ddd); margin: 2em 0; }
    a { color: var(--vscode-textLink-foreground, #0066cc); }
    blockquote {
      border-left: 3px solid var(--vscode-editorWidget-border, #ccc);
      margin: 1em 0;
      padding: .2em 1em;
      color: var(--vscode-descriptionForeground, #666);
    }
  </style>
</head>
<body>
${body}
</body>
</html>`;
}

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function inline(text: string): string {
  // Every emitted HTML fragment goes through ONE shared placeholder
  // table so the final auto-escape pass cannot mangle the tags we
  // just emitted. Earlier versions only stashed `<code>` and let
  // `<strong>` / `<a>` flow through into the escape pass, which
  // turned them into `&lt;strong&gt;…&lt;/strong&gt;` literals on
  // screen. PUA sentinels round-trip the escape pass cleanly
  // because they're neither `<`, `>`, `&`, `"`, nor `'`.
  const segments: string[] = [];
  const PhOpen = "\u{E000}";
  const PhClose = "\u{E001}";
  const stash = (html: string): string => {
    segments.push(html);
    return `${PhOpen}${segments.length - 1}${PhClose}`;
  };

  // 1. Inline code — process FIRST so escaping inside the backticks
  //    is preserved as-is and any `**` / `[` / `<` inside code never
  //    gets re-interpreted.
  //
  //    Two delimiter forms, in priority order:
  //
  //      (a) Doubled-backtick `` `` `…` `` `` — the GitHub trick for
  //          embedding a literal single ` inside a code span (used
  //          by `media/notation-guide.md`'s "アクセント分解" table
  //          for the `` `グレーブ` `` row). The body is trimmed of
  //          one leading/trailing space if present, mirroring
  //          CommonMark.
  //      (b) Single-backtick `…` — the ordinary case.
  //
  //    Doubled MUST run first, otherwise the single-backtick regex
  //    eats `` `` `` as an empty code span and leaves the inner
  //    backtick orphaned.
  const withDoubleCode = text.replace(/``\s?([^`]+?)\s?``/g, (_, body) =>
    stash(`<code>${escapeHtml(body)}</code>`),
  );
  const withCode = withDoubleCode.replace(/`([^`]+)`/g, (_, body) =>
    stash(`<code>${escapeHtml(body)}</code>`),
  );
  // 2. Markdown links [text](url) → stash as <a> tag.
  const withLinks = withCode.replace(/\[([^\]]+)\]\(([^)]+)\)/g, (_, label, url) =>
    stash(`<a href="${escapeHtml(url)}">${escapeHtml(label)}</a>`),
  );
  // 3. Bold **text** → stash as <strong> tag.
  const withBold = withLinks.replace(/\*\*([^*]+)\*\*/g, (_, body) =>
    stash(`<strong>${escapeHtml(body)}</strong>`),
  );
  // 4. Auto-escape any remaining < > & " ' in plain runs. Our
  //    PUA sentinels are not in this character class so they pass
  //    through untouched.
  const escaped = withBold.replace(/[<>&"']/g, (ch) => {
    return { "<": "&lt;", ">": "&gt;", "&": "&amp;", '"': "&quot;", "'": "&#39;" }[ch] ?? ch;
  });
  // 5. Restore every stashed HTML fragment in a single pass.
  return escaped.replace(/\u{E000}(\d+)\u{E001}/gu, (_, idx) => segments[Number(idx)] ?? "");
}

function mdToHtml(md: string): string {
  const lines = md.split(/\r?\n/);
  const out: string[] = [];
  let i = 0;
  let inCode = false;
  let codeBuf: string[] = [];

  const flushParagraph = (buf: string[]) => {
    if (buf.length === 0) {
      return;
    }
    out.push(`<p>${inline(buf.join(" ").trim())}</p>`);
    buf.length = 0;
  };

  const paraBuf: string[] = [];

  // Helper: bounded line read with explicit narrowing under
  // `noUncheckedIndexedAccess`. Returns "" past EOF so the regex
  // checks fall through naturally instead of crashing on undefined.
  const at = (idx: number): string => lines[idx] ?? "";

  while (i < lines.length) {
    const line = at(i);

    if (inCode) {
      if (/^```/.test(line)) {
        out.push(`<pre><code>${escapeHtml(codeBuf.join("\n"))}</code></pre>`);
        codeBuf = [];
        inCode = false;
      } else {
        codeBuf.push(line);
      }
      i++;
      continue;
    }

    if (/^```/.test(line)) {
      flushParagraph(paraBuf);
      inCode = true;
      i++;
      continue;
    }

    // Match only the prefix so the regex stays unambiguous —
    // `\s+(.*)` flags as polynomial-redos because `.` overlaps `\s`,
    // forcing quadratic backtracking on long all-whitespace tails.
    // Body comes from a plain slice + trimStart instead.
    const heading = /^(#{1,6})\s/.exec(line);
    if (heading) {
      flushParagraph(paraBuf);
      const level = heading[1]?.length ?? 1;
      const body = line.slice(heading[0].length).trimStart();
      out.push(`<h${level}>${inline(body)}</h${level}>`);
      i++;
      continue;
    }

    if (/^---+\s*$/.test(line)) {
      flushParagraph(paraBuf);
      out.push("<hr>");
      i++;
      continue;
    }

    if (/^\|.*\|\s*$/.test(line) && i + 1 < lines.length && /^\|[-\s:|]+\|\s*$/.test(at(i + 1))) {
      flushParagraph(paraBuf);
      const header = parseTableRow(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && /^\|.*\|\s*$/.test(at(i))) {
        rows.push(parseTableRow(at(i)));
        i++;
      }
      out.push(renderTable(header, rows));
      continue;
    }

    if (/^[-*]\s+/.test(line)) {
      flushParagraph(paraBuf);
      const items: string[] = [];
      while (i < lines.length && /^[-*]\s+/.test(at(i))) {
        items.push(inline(at(i).replace(/^[-*]\s+/, "")));
        i++;
      }
      out.push(`<ul>${items.map((it) => `<li>${it}</li>`).join("")}</ul>`);
      continue;
    }

    if (/^\d+\.\s+/.test(line)) {
      flushParagraph(paraBuf);
      const items: string[] = [];
      while (i < lines.length && /^\d+\.\s+/.test(at(i))) {
        items.push(inline(at(i).replace(/^\d+\.\s+/, "")));
        i++;
      }
      out.push(`<ol>${items.map((it) => `<li>${it}</li>`).join("")}</ol>`);
      continue;
    }

    if (/^\s*$/.test(line)) {
      flushParagraph(paraBuf);
      i++;
      continue;
    }

    paraBuf.push(line);
    i++;
  }
  flushParagraph(paraBuf);
  return out.join("\n");
}

function parseTableRow(line: string): string[] {
  // GFM table cells can contain a literal pipe by escaping it as
  // `\|`. The naive `split("|")` would treat the escaped pipe as a
  // column boundary, so substitute a Unicode PUA sentinel during the
  // split and restore the literal `|` in each cell afterwards. The
  // sentinel sits in the BMP private-use area — never produced by
  // any markdown content we accept — so the round-trip is reliable.
  const PipeSentinel = "\u{F8FF}";
  return line
    .replace(/\\\|/g, PipeSentinel)
    .replace(/^\||\|\s*$/g, "")
    .split("|")
    .map((c) => c.trim().replaceAll(PipeSentinel, "|"));
}

function renderTable(header: string[], rows: string[][]): string {
  const head = `<thead><tr>${header.map((h) => `<th>${inline(h)}</th>`).join("")}</tr></thead>`;
  const body = `<tbody>${rows
    .map((r) => `<tr>${r.map((c) => `<td>${inline(c)}</td>`).join("")}</tr>`)
    .join("")}</tbody>`;
  return `<table>${head}${body}</table>`;
}
