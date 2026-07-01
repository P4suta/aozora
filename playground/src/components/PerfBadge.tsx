import { createSignal, For, onCleanup, Show } from 'solid-js';
import type { ProfilePhaseEntry } from '../editor';
import { t } from '../i18n';

interface PerfBadgeProps {
  parseDurationMs: number;
  byteLen: number;
  profile: ProfilePhaseEntry[];
}

function fmtMs(ms: number): string {
  if (ms <= 0) return '—';
  if (ms < 1) return `${ms.toFixed(2)} ms`;
  if (ms < 10) return `${ms.toFixed(2)} ms`;
  return `${ms.toFixed(1)} ms`;
}

function fmtMbps(bytes: number, ms: number): string {
  if (ms <= 0 || bytes === 0) return '—';
  const mbps = bytes / 1_000_000 / (ms / 1000);
  if (mbps >= 100) return `${mbps.toFixed(0)} MB/s`;
  if (mbps >= 10) return `${mbps.toFixed(1)} MB/s`;
  return `${mbps.toFixed(2)} MB/s`;
}

function summary(props: PerfBadgeProps): string {
  if (props.byteLen === 0) return '—';
  return `${fmtMs(props.parseDurationMs)} · ${fmtMbps(props.byteLen, props.parseDurationMs)} · ${props.byteLen.toLocaleString()} bytes`;
}

export default function PerfBadge(props: PerfBadgeProps) {
  const [open, setOpen] = createSignal(false);
  let rootEl: HTMLSpanElement | undefined;

  function handleClickOutside(event: MouseEvent) {
    if (!open()) return;
    if (rootEl && !rootEl.contains(event.target as Node)) setOpen(false);
  }
  document.addEventListener('mousedown', handleClickOutside);
  onCleanup(() => document.removeEventListener('mousedown', handleClickOutside));

  return (
    <span class="perf-badge-root" ref={rootEl}>
      <button
        type="button"
        class="perf-badge"
        onClick={() => setOpen((v) => !v)}
        title={t('perfExpand')}
        aria-haspopup="true"
        aria-expanded={open()}
      >
        {summary(props)}
        <Show when={props.profile.length > 0}>
          <span class="perf-badge-chevron" aria-hidden="true">
            ▾
          </span>
        </Show>
      </button>
      <Show when={open() && props.profile.length > 0}>
        <div class="perf-popover" role="dialog" aria-label={t('perfProfile')}>
          <div class="perf-popover-header">
            <span>{t('perfHeader')}</span>
            <span class="perf-popover-sub">{props.byteLen.toLocaleString()} bytes</span>
          </div>
          <table class="perf-table">
            <thead>
              <tr>
                <th>{t('perfColMethod')}</th>
                <th>{t('perfColTime')}</th>
                <th>{t('perfColThroughput')}</th>
              </tr>
            </thead>
            <tbody>
              <For each={props.profile}>
                {(p) => (
                  <tr>
                    <td>
                      <code>{p.name}</code>
                    </td>
                    <td class="perf-num">{fmtMs(p.duration_ms)}</td>
                    <td class="perf-num">{fmtMbps(props.byteLen, p.duration_ms)}</td>
                  </tr>
                )}
              </For>
              <tr class="perf-total">
                <td>{t('perfTotal')}</td>
                <td class="perf-num">
                  {fmtMs(props.profile.reduce((acc, p) => acc + p.duration_ms, 0))}
                </td>
                <td class="perf-num">
                  {fmtMbps(
                    props.byteLen,
                    props.profile.reduce((acc, p) => acc + p.duration_ms, 0),
                  )}
                </td>
              </tr>
            </tbody>
          </table>
          <div class="perf-popover-footer">
            <span class="perf-popover-sub">
              {t('perfFooterPre')} <code>performance.now()</code>
              {t('perfFooterMid')} <code>parse + toHtml</code>
              {t('perfFooterPost')}
            </span>
          </div>
        </div>
      </Show>
    </span>
  );
}
