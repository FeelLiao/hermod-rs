// 高频进度独立订阅，避免每次采样都重新渲染整个设置和导航树。
import { useSyncExternalStore } from "react";
import { type Status } from "./api";
import { type Locale, translator } from "./i18n";
import { ProgressBar } from "./ui";

let current: Status | null = null;
const listeners = new Set<() => void>();
export function publishStatus(value: Status | null) {
  if (JSON.stringify(current) === JSON.stringify(value)) return;
  current = value;
  listeners.forEach((listener) => listener());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function statusSummary(value: Status | null) {
  if (!value) return "";
  const { bytes: _bytes, elapsed: _elapsed, rtt_ms: _rtt, ...summary } = value;
  return JSON.stringify(summary);
}
export function TransferMetrics({
  locale,
  size,
}: {
  locale: Locale;
  size: (n: number | null) => string;
}) {
  const status = useSyncExternalStore(subscribe, () => current);
  const t = translator(locale);
  if (!status) return null;
  const value =
    status.total === null
      ? undefined
      : status.total === 0
        ? status.digest
          ? 1
          : 0
        : Math.min(1, status.bytes / status.total);
  return (
    <>
      <ProgressBar aria-label={t("Transfer progress")} value={value} />
      <div className="summary">
        <span>
          {size(status.bytes)} / {size(status.total)}
        </span>
        <span>
          {status.transport === "quic"
            ? "QUIC"
            : status.transport === "wireguard"
              ? "WireGuard"
              : t("Connecting")}
        </span>
      </div>
      <div className="metrics">
        <div>
          <strong>
            {size(status.elapsed > 0 ? status.bytes / status.elapsed : 0)}/s
          </strong>
          <span>{t("Average payload")}</span>
        </div>
        <div>
          <strong>
            {status.rtt_ms == null ? "—" : `${status.rtt_ms.toFixed(0)} ms`}
          </strong>
          <span>{t("Network RTT")}</span>
        </div>
      </div>
    </>
  );
}
