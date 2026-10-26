import { invoke } from "@tauri-apps/api/core";
import type { Language, Locale } from "./i18n";
export interface Settings {
  connection: {
    transport: string;
    mtu: string;
    congestion: string;
    limit_mbps: string;
  };
  receive_transport: string;
  server: string;
  words: number;
  verbose: string;
  quiet: boolean;
  theme: string;
  language: Language;
  destination: string | null;
}
export interface Selection {
  id: string;
  name: string;
  size: number | null;
}
export interface Received {
  id: string;
  name: string;
  size: number;
  sha256: string;
}
export interface Status {
  phase: string;
  code: string | null;
  bytes: number;
  total: number | null;
  elapsed: number;
  done: boolean;
  cancelled: boolean;
  error: string | null;
  export_error?: string | null;
  digest: string | null;
  rtt_ms: number | null;
  transport: string | null;
  received_id: string | null;
}
export interface Bootstrap {
  version: string;
  ipv6: string[];
  platform: string;
  color_seed?: string;
  settings: Settings;
  trusted_servers: Record<string, string>;
  settings_saved: boolean;
}
export const api = {
  locale: (locale: Locale) => invoke<void>("set_locale", { locale }),
  bootstrap: () => invoke<Bootstrap>("bootstrap"),
  save: (settings: Settings) => invoke<void>("save_settings", { settings }),
  choose: () => invoke<Selection | null>("choose_source"),
  destination: () => invoke<string | null>("choose_destination"),
  clearDestination: () => invoke<void>("clear_destination"),
  start: (sourceId: string | null, code: string | null, text: string | null) =>
    invoke<void>("start_transfer", { sourceId, code, text }),
  status: () => invoke<Status | null>("status"),
  cancel: () => invoke<void>("cancel_transfer"),
  copy: () => invoke<void>("copy_code"),
  files: () => invoke<Received[]>("list_received"),
  export: (id: string, useDefault = false) =>
    invoke<boolean>("export_received", { id, useDefault }),
  remove: (id: string) => invoke<void>("delete_received", { id }),
  probe: (server: string) => invoke<string>("probe_server", { server }),
  trust: (server: string, fingerprint: string) =>
    invoke<void>("trust_server", { server, fingerprint }),
  logs: () => invoke<string[]>("get_logs"),
};
