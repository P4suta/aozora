import { type ChildProcessWithoutNullStreams, spawn } from "node:child_process";

import * as vscode from "vscode";

import {
  decodeLintSource,
  formatLintProcessResult,
  WORKSPACE_LINT_SOURCE_LIMIT,
  workspaceLintSourceSizeSupported,
} from "./workspaceLintLogic";

const PROCESS_CONCURRENCY = 4;
const PROCESS_OUTPUT_LIMIT = 1_000_000;

interface ProcessResult {
  readonly code: number;
  readonly stdout: string;
  readonly stderr: string;
}

export class WorkspaceLintTerminal implements vscode.Pseudoterminal {
  private readonly writeEmitter = new vscode.EventEmitter<string>();
  private readonly closeEmitter = new vscode.EventEmitter<number | undefined>();
  private readonly children = new Set<ChildProcessWithoutNullStreams>();
  private cancelled = false;
  private finished = false;

  readonly onDidWrite = this.writeEmitter.event;
  readonly onDidClose = this.closeEmitter.event;

  constructor(
    private readonly executable: string,
    private readonly files: readonly vscode.Uri[],
    private readonly cwd: string,
  ) {}

  open(): void {
    void this.run();
  }

  close(): void {
    this.cancelled = true;
    for (const child of this.children) {
      child.kill();
    }
    this.finish();
  }

  private async run(): Promise<void> {
    this.write(`${this.files.length} Aozora text file(s)\n`);
    let next = 0;
    let failed = false;
    const worker = async () => {
      while (!this.cancelled) {
        const index = next++;
        const uri = this.files[index];
        if (!uri) {
          return;
        }
        try {
          const metadata = await vscode.workspace.fs.stat(uri);
          if (!workspaceLintSourceSizeSupported(metadata.size)) {
            throw new Error(
              `source exceeds the supported ${WORKSPACE_LINT_SOURCE_LIMIT}-byte limit`,
            );
          }
          const result = await this.runOne(uri.fsPath);
          if (this.cancelled) {
            return;
          }
          let source = "";
          if (result.stderr.trim().length > 0) {
            const bytes = await vscode.workspace.fs.readFile(uri);
            source = decodeLintSource(bytes);
          }
          const formatted = formatLintProcessResult(result, uri.fsPath, source);
          for (const line of formatted.lines) {
            this.write(`${line}\n`);
          }
          if (formatted.failed) {
            failed = true;
          }
        } catch (error) {
          failed = true;
          const message = error instanceof Error ? error.message : String(error);
          this.write(`${uri.fsPath}: aozora lint failed: ${message}\n`);
        }
      }
    };
    const workers = Array.from(
      { length: Math.min(PROCESS_CONCURRENCY, this.files.length) },
      worker,
    );
    await Promise.all(workers);
    this.finish(failed ? 1 : 0);
  }

  private runOne(file: string): Promise<ProcessResult> {
    return new Promise((resolve, reject) => {
      const child = spawn(
        this.executable,
        [
          "lint",
          "--encoding",
          "auto",
          "--format",
          "json",
          "--color",
          "never",
          "--lang",
          "en",
          file,
        ],
        {
          cwd: this.cwd,
          shell: false,
          windowsHide: true,
        },
      );
      this.children.add(child);
      const stdout: Buffer[] = [];
      const stderr: Buffer[] = [];
      let outputBytes = 0;
      let outputExceeded = false;
      const append = (chunks: Buffer[], chunk: Buffer): void => {
        if (outputExceeded) {
          return;
        }
        outputBytes += chunk.byteLength;
        if (outputBytes > PROCESS_OUTPUT_LIMIT) {
          outputExceeded = true;
          child.kill();
          return;
        }
        chunks.push(chunk);
      };
      child.stdout.on("data", (chunk: Buffer) => {
        append(stdout, chunk);
      });
      child.stderr.on("data", (chunk: Buffer) => {
        append(stderr, chunk);
      });
      child.once("error", (error) => {
        this.children.delete(child);
        reject(error);
      });
      child.once("close", (code) => {
        this.children.delete(child);
        if (outputExceeded) {
          reject(new Error("process output exceeded the supported limit"));
          return;
        }
        resolve({
          code: code ?? 1,
          stdout: Buffer.concat(stdout).toString("utf8"),
          stderr: Buffer.concat(stderr).toString("utf8"),
        });
      });
    });
  }

  private write(value: string): void {
    this.writeEmitter.fire(value.replace(/\r?\n/g, "\r\n"));
  }

  private finish(code?: number): void {
    if (this.finished) {
      return;
    }
    this.finished = true;
    this.closeEmitter.fire(code);
  }
}
