import React, { useCallback, useEffect, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  Accordion,
  AccordionHeader,
  AccordionItem,
  AccordionPanel,
  Badge,
  Button,
  Card,
  Checkbox,
  Dialog,
  DialogActions,
  DialogBody,
  DialogContent,
  DialogSurface,
  DialogTitle,
  Field,
  UiProvider,
  Input,
  MessageBar,
  MessageBarBody,
  Select,
  Spinner,
  Tab,
  TabList,
  Textarea,
  Title1,
  Title2,
} from "./ui";
import {
  ArrowDownload24Regular,
  ArrowUpload24Regular,
  Copy24Regular,
  Document24Regular,
  FolderOpen24Regular,
  Settings24Regular,
  ShieldCheckmark24Regular,
  Delete24Regular,
} from "@fluentui/react-icons";
import {
  api,
  type Bootstrap,
  type Received,
  type Selection,
  type Settings,
  type Status,
} from "./api";
import { resolveLocale, runtimeText, translator, type Language } from "./i18n";
import "./style.css";
import { MotionRegion, Presence, transition } from "./motion";
import {
  publishStatus,
  statusSummary,
  TransferMetrics,
} from "./transfer-status";
function App() {
  const [info, setInfo] = useState<Bootstrap | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [preferredLanguages, setPreferredLanguages] = useState(() => [
    ...navigator.languages,
  ]);
  const locale = resolveLocale(
    settings?.language ?? "system",
    preferredLanguages,
  );
  const t = translator(locale);
  const protocolName = (value?: string | null) =>
    value === "auto"
      ? t("Auto")
      : value === "wireguard"
        ? "WireGuard"
        : value === "quic"
          ? "QUIC"
          : t("Connecting");
  const size = (n: number | null) =>
    n === null
      ? t("Unknown size")
      : n < 1024
        ? `${n} B`
        : n < 1048576
          ? `${(n / 1024).toFixed(1)} KiB`
          : n < 1073741824
            ? `${(n / 1048576).toFixed(1)} MiB`
            : `${(n / 1073741824).toFixed(2)} GiB`;
  useEffect(() => {
    const changed = () => setPreferredLanguages([...navigator.languages]);
    window.addEventListener("languagechange", changed);
    return () => window.removeEventListener("languagechange", changed);
  }, []);
  useEffect(() => {
    document.documentElement.lang = locale;
    void api
      .locale(locale)
      .catch((error) => setNotice({ text: String(error), error: true }));
  }, [locale]);
  const [page, setPage] = useState("transfer");
  const [mode, setMode] = useState("file");
  const [source, setSource] = useState<Selection | null>(null);
  const [code, setCode] = useState("");
  const [text, setText] = useState("");
  const [status, setStatus] = useState<Status | null>(null);
  const updateStatus = useCallback((value: Status | null) => {
    publishStatus(value);
    setStatus((current) =>
      statusSummary(current) === statusSummary(value) ? current : value,
    );
  }, []);
  const [busy, setBusy] = useState(false);
  const operation = useRef(false);
  const [notice, setNotice] = useState<{ text: string; error: boolean } | null>(
    null,
  );
  const [files, setFiles] = useState<Received[]>([]);
  const [logs, setLogs] = useState<string[]>([]);
  const [removing, setRemoving] = useState<Received | null>(null);
  const [fingerprint, setFingerprint] = useState("");
  const [trusting, setTrusting] = useState(false);
  const [systemDark, setSystemDark] = useState(
    matchMedia("(prefers-color-scheme: dark)").matches,
  );
  const completed = useRef<string | null>(null);
  const saved = useRef<Settings | null>(null);
  const running = !!status && !status.done;
  const report = (message: string) => {
    if (!saved.current?.quiet) setNotice({ text: message, error: false });
  };
  async function work(action: () => Promise<void>) {
    if (operation.current) return;
    operation.current = true;
    setBusy(true);
    try {
      await action();
    } catch (e) {
      setNotice({ text: String(e), error: true });
    } finally {
      operation.current = false;
      setBusy(false);
    }
  }
  useEffect(() => {
    const media = matchMedia("(prefers-color-scheme: dark)");
    const listener = () => setSystemDark(media.matches);
    media.addEventListener("change", listener);
    void api
      .bootstrap()
      .then(async (value) => {
        if (!value.settings_saved) {
          const budget = localStorage.getItem("hermod-budget"),
            theme = localStorage.getItem("hermod-theme");
          if (
            budget !== null &&
            Number.isFinite(Number(budget)) &&
            Number(budget) >= 0
          )
            value.settings.connection.limit_mbps = budget;
          if (theme && ["system", "light", "dark"].includes(theme))
            value.settings.theme = theme;
          if (budget !== null || theme !== null) await api.save(value.settings);
        }
        setInfo(value);
        setSettings(value.settings);
        saved.current = value.settings;
      })
      .catch((e) => setNotice({ text: String(e), error: true }));
    return () => media.removeEventListener("change", listener);
  }, []);
  useEffect(() => {
    let stopped = false,
      polling = false;
    async function poll() {
      if (polling) return;
      polling = true;
      try {
        const value = await api.status();
        if (stopped) return;
        updateStatus(value);
        if (
          value?.done &&
          value.received_id &&
          completed.current !== value.received_id &&
          !operation.current
        ) {
          completed.current = value.received_id;
          setFiles(await api.files());
        }
      } catch (e) {
        if (!stopped) setNotice({ text: String(e), error: true });
      } finally {
        polling = false;
      }
    }
    const timer = setInterval(() => void poll(), 300);
    void poll();
    return () => {
      stopped = true;
      clearInterval(timer);
    };
  }, []);
  useEffect(() => {
    if (page === "inbox") void work(async () => setFiles(await api.files()));
    if (page === "logs")
      void api
        .logs()
        .then(setLogs)
        .catch((e) => setNotice({ text: String(e), error: true }));
  }, [page]);
  const dark =
    settings?.theme === "dark" || (settings?.theme !== "light" && systemDark);
  const patch = (value: Partial<Settings>) =>
    setSettings((current) => (current ? { ...current, ...value } : current));
  const network = (value: Partial<Settings["connection"]>) =>
    setSettings((current) =>
      current
        ? { ...current, connection: { ...current.connection, ...value } }
        : current,
    );
  const algorithms =
    settings?.connection.transport === "quic"
      ? ["auto", "bbr", "cubic", "newreno"]
      : ["auto", "cubic", "reno"];
  async function start() {
    await work(async () => {
      if (mode === "receive" && !/^\d+-[a-z]+(?:-[a-z]+)*$/.test(code.trim()))
        throw new Error("Enter a valid transfer code.");
      await api.start(
        mode === "file" ? (source?.id ?? null) : null,
        mode === "receive" ? code.trim() : null,
        mode === "text" ? text : null,
      );
      updateStatus(await api.status());
      completed.current = null;
      if (mode === "file") setSource(null);
    });
  }
  return (
    <UiProvider
      android={info?.platform === "android"}
      seed={info?.color_seed}
      dark={dark}
      className="shell"
    >
      <aside className="rail">
        <div className="brand">
          <span className="brand-icon">H</span>
          <span>Hermod</span>
        </div>
        <TabList
          vertical
          aria-label={t("Main navigation")}
          selectedValue={page}
          onTabSelect={(_, d) => transition(() => setPage(String(d.value)))}
        >
          <Tab value="transfer" icon={<ArrowUpload24Regular />}>
            {t("Transfer")}
          </Tab>
          <Tab value="inbox" icon={<ArrowDownload24Regular />}>
            {t("Inbox")}
          </Tab>
          <Tab value="settings" icon={<Settings24Regular />}>
            {t("Settings")}
          </Tab>
          <Tab value="logs" icon={<Document24Regular />}>
            {t("Logs")}
          </Tab>
        </TabList>
        <div className="rail-footer">
          <ShieldCheckmark24Regular />
          <span>
            {t("Encrypted peer to peer")}
            <br />
            {t("IPv4 LAN / IPv6 · No file relay")}
          </span>
        </div>
      </aside>
      <main>
        <MotionRegion motionKey={page} className="page-content">
          <header>
            <div>
              <Title1>
                {page === "transfer"
                  ? t("A direct connection.")
                  : page === "inbox"
                    ? t("Your verified files.")
                    : page === "settings"
                      ? t("Make it yours.")
                      : t("Connection logs.")}
              </Title1>
              <p className="subtle">
                {page === "transfer"
                  ? t(
                      "Send a file or a message. Share the code. Stay in control.",
                    )
                  : t("Hermod · private file transfer")}
              </p>
            </div>
            <Badge
              appearance="tint"
              color={
                info?.ipv6.length || info?.ipv4?.length ? "success" : "warning"
              }
            >
              {info?.ipv6.length
                ? t("IPv6 available")
                : info?.ipv4?.length
                  ? t("IPv4 LAN available")
                  : t("No local network address")}
            </Badge>
          </header>
          <Presence visible={!!notice}>
            {notice && (
              <MessageBar intent={notice.error ? "error" : "success"}>
                <MessageBarBody>
                  {runtimeText(notice.text, locale, notice.error)}
                </MessageBarBody>
                <Button
                  appearance="transparent"
                  aria-label={t("Dismiss message")}
                  onClick={() => setNotice(null)}
                >
                  {t("Dismiss")}
                </Button>
              </MessageBar>
            )}
          </Presence>
          {!settings && <Spinner label={t("Loading settings")} />}
          {settings && page === "transfer" && (
            <div className="transfer-grid">
              <Card className="panel">
                <TabList
                  aria-label={t("Transfer type")}
                  selectedValue={mode}
                  onTabSelect={(_, d) =>
                    transition(() => setMode(String(d.value)))
                  }
                  disabled={running || busy}
                >
                  <Tab value="file">{t("File")}</Tab>
                  <Tab value="text">{t("Text")}</Tab>
                  <Tab value="receive">{t("Receive")}</Tab>
                </TabList>
                <MotionRegion motionKey={mode} className="mode-content">
                  {mode === "file" && (
                    <>
                      <Title2>{t("Choose what to send")}</Title2>
                      <button
                        className="file-picker"
                        disabled={running || busy}
                        onClick={() =>
                          void work(async () => {
                            const selected = await api.choose();
                            if (selected) setSource(selected);
                          })
                        }
                      >
                        <FolderOpen24Regular />
                        <strong>{source?.name ?? t("Choose file")}</strong>
                        <span>
                          {source
                            ? size(source.size)
                            : t("Open the system file picker")}
                        </span>
                      </button>
                    </>
                  )}
                  {mode === "text" && (
                    <Field
                      label={t("Message")}
                      hint={t("Up to 1 MiB; received as a verified text file")}
                    >
                      <Textarea
                        resize="vertical"
                        rows={8}
                        value={text}
                        onChange={(_, d) => setText(d.value)}
                        disabled={running || busy}
                      />
                    </Field>
                  )}
                  {mode === "receive" && (
                    <>
                      <Title2>{t("Connect with a code")}</Title2>
                      <Field
                        label={t("Transfer code")}
                        hint={t("Ask your sender for their complete code")}
                      >
                        <Input
                          value={code}
                          onChange={(_, d) => setCode(d.value)}
                          disabled={running || busy}
                          placeholder="1234-word-word-word"
                        />
                      </Field>
                      <p>
                        {t(
                          "Verified files stay in your private inbox. Save a copy anywhere.",
                        )}
                      </p>
                    </>
                  )}
                </MotionRegion>
                <div className="summary">
                  <span>
                    {protocolName(
                      mode === "receive"
                        ? saved.current?.receive_transport
                        : saved.current?.connection.transport,
                    )}{" "}
                    ·{" "}
                    {saved.current?.connection.limit_mbps === "0"
                      ? t("Unlimited")
                      : t("{limit} Mbps cap", {
                          limit: saved.current?.connection.limit_mbps ?? "0",
                        })}
                  </span>
                  <Button
                    appearance="subtle"
                    onClick={() => transition(() => setPage("settings"))}
                  >
                    {t("Adjust")}
                  </Button>
                </div>
                <Button
                  appearance="primary"
                  size="large"
                  icon={
                    mode === "receive" ? (
                      <ArrowDownload24Regular />
                    ) : (
                      <ArrowUpload24Regular />
                    )
                  }
                  disabled={
                    busy ||
                    running ||
                    (mode === "file" && !source) ||
                    (mode === "text" && !text)
                  }
                  onClick={() => void start()}
                >
                  {mode === "receive"
                    ? t("Connect & receive")
                    : t("Create transfer code")}
                </Button>
                <p className="subtle">
                  {t(
                    "Use the same IPv4 LAN or reachable IPv6. Keep the transfer code private.",
                  )}
                </p>
              </Card>
              <Card className="panel">
                <Badge
                  appearance="tint"
                  color={
                    status?.error
                      ? "danger"
                      : status?.digest
                        ? "success"
                        : "brand"
                  }
                >
                  {running
                    ? t("ACTIVE")
                    : status?.digest
                      ? t("VERIFIED")
                      : t("READY")}
                </Badge>
                <Title2>
                  {status
                    ? runtimeText(status.phase, locale)
                    : t("Ready when you are.")}
                </Title2>
                {!status && (
                  <p className="subtle">
                    {t(
                      "Your file travels directly between devices. The signaling server only helps them meet.",
                    )}
                  </p>
                )}
                {status?.export_error && (
                  <MessageBar intent="error">
                    <MessageBarBody>
                      {runtimeText(status.export_error, locale, true)}
                    </MessageBarBody>
                  </MessageBar>
                )}
                {status?.error && (
                  <MessageBar intent="error">
                    <MessageBarBody>
                      {runtimeText(status.error, locale, true)}
                    </MessageBarBody>
                  </MessageBar>
                )}
                {status?.code && (
                  <div className="code-box">
                    <span>{t("Share this code")}</span>
                    <code data-testid="transfer-code">{status.code}</code>
                    <Button
                      icon={<Copy24Regular />}
                      onClick={() =>
                        void work(async () => {
                          await api.copy();
                          report("Transfer code copied.");
                        })
                      }
                    >
                      {t("Copy code")}
                    </Button>
                  </div>
                )}
                <TransferMetrics locale={locale} size={size} />
                {status?.digest && (
                  <div className="digest">
                    <ShieldCheckmark24Regular />
                    <span>{t("SHA-256 verified")}</span>
                    <code>{status.digest}</code>
                  </div>
                )}
                {running && (
                  <Button
                    onClick={() =>
                      void work(async () => {
                        await api.cancel();
                        updateStatus(await api.status());
                      })
                    }
                  >
                    {t("Cancel transfer")}
                  </Button>
                )}
              </Card>
            </div>
          )}
          {page === "inbox" && (
            <>
              <div className="toolbar">
                <span>
                  {t("{count} verified files", { count: files.length })}
                </span>
                <Button
                  onClick={() =>
                    void work(async () => setFiles(await api.files()))
                  }
                >
                  {t("Refresh")}
                </Button>
              </div>
              {files.length === 0 && (
                <Card className="panel">
                  <Title2>{t("Your inbox is empty")}</Title2>
                  <p>{t("Completed, verified transfers will appear here.")}</p>
                </Card>
              )}
              {files.map((file) => (
                <Card key={file.id} className="inbox-row">
                  <Document24Regular />
                  <div className="file-detail">
                    <strong>{file.name}</strong>
                    <span className="subtle">
                      {size(file.size)} · {t("SHA-256 verified")}
                    </span>
                  </div>
                  <Button
                    disabled={busy || running}
                    icon={<ArrowDownload24Regular />}
                    onClick={() =>
                      void work(async () => {
                        if (await api.export(file.id))
                          report("Verified copy saved.");
                      })
                    }
                  >
                    {t("Save a copy")}
                  </Button>
                  <Button
                    disabled={busy || running}
                    icon={<Delete24Regular />}
                    aria-label={t("Remove {name}", { name: file.name })}
                    onClick={() => setRemoving(file)}
                  />
                </Card>
              ))}
              <p className="subtle">
                {t(
                  "Export important files before uninstalling. Removing an inbox copy does not remove exported copies.",
                )}
              </p>
            </>
          )}
          {settings && page === "settings" && (
            <div className="settings-grid">
              <Card className="panel">
                <Title2>{t("Transfer preferences")}</Title2>
                {(settings.version ?? 1) < 2 && (
                  <MessageBar intent="info">
                    <MessageBarBody>
                      {t(
                        "New defaults: QUIC and unlimited speed. Your saved choices have been kept.",
                      )}
                    </MessageBarBody>
                    <Button
                      onClick={() =>
                        patch({
                          version: 2,
                          connection: {
                            transport: "quic",
                            limit_mbps: "0",
                            mtu: "auto",
                            congestion: "auto",
                          },
                        })
                      }
                    >
                      {t("Use new defaults")}
                    </Button>
                    <Button onClick={() => patch({ version: 2 })}>
                      {t("Keep my settings")}
                    </Button>
                  </MessageBar>
                )}
                <Field label={t("Send protocol")}>
                  <Select
                    value={settings.connection.transport}
                    onChange={(_, d) =>
                      network({ transport: d.value, congestion: "auto" })
                    }
                  >
                    <option value="wireguard">WireGuard</option>
                    <option value="quic">{t("QUIC (default)")}</option>
                  </Select>
                </Field>
                <Field label={t("Receive protocol")}>
                  <Select
                    value={settings.receive_transport}
                    onChange={(_, d) => patch({ receive_transport: d.value })}
                  >
                    <option value="auto">
                      {t("Auto — follow the sender")}
                    </option>
                    <option value="wireguard">{t("WireGuard only")}</option>
                    <option value="quic">{t("QUIC only")}</option>
                  </Select>
                </Field>
                <Field
                  label={t("Payload limit (Mbps)")}
                  hint={t(
                    "0 = unlimited. Start at 70–85% of your upload speed.",
                  )}
                >
                  <Input
                    value={settings.connection.limit_mbps}
                    type="number"
                    min={0}
                    step="0.1"
                    onChange={(_, d) => network({ limit_mbps: d.value })}
                  />
                </Field>
                <Field label={t("Code length (words)")}>
                  <Input
                    type="number"
                    min={1}
                    max={32}
                    value={String(settings.words)}
                    onChange={(_, d) => patch({ words: Number(d.value) })}
                  />
                </Field>
                <Accordion collapsible>
                  <AccordionItem value="network">
                    <AccordionHeader>
                      {t("Advanced network settings")}
                    </AccordionHeader>
                    <AccordionPanel className="field-stack">
                      <Field
                        label="MTU"
                        hint={t(
                          "auto, or bytes: WireGuard inner IP; QUIC UDP payload cap (1200–1452).",
                        )}
                      >
                        <Input
                          value={settings.connection.mtu}
                          onChange={(_, d) => network({ mtu: d.value })}
                        />
                      </Field>
                      <Field label={t("Congestion control")}>
                        <Select
                          value={settings.connection.congestion}
                          onChange={(_, d) => network({ congestion: d.value })}
                        >
                          {algorithms.map((a) => (
                            <option key={a} value={a}>
                              {a === "auto" ? t("Auto") : a.toUpperCase()}
                            </option>
                          ))}
                        </Select>
                      </Field>
                    </AccordionPanel>
                  </AccordionItem>
                </Accordion>
                <Field
                  label={t("Default export folder")}
                  hint={t("A verified inbox copy is always retained.")}
                >
                  <div className="actions">
                    <Input
                      readOnly
                      value={settings.destination ?? t("Private inbox only")}
                    />
                    <Button
                      onClick={() =>
                        void work(async () => {
                          const value = await api.destination();
                          if (value) {
                            patch({ destination: value });
                            if (saved.current)
                              saved.current = {
                                ...saved.current,
                                destination: value,
                              };
                          }
                        })
                      }
                    >
                      {t("Choose folder")}
                    </Button>
                    <Button
                      onClick={() =>
                        void work(async () => {
                          await api.clearDestination();
                          patch({ destination: null });
                          if (saved.current)
                            saved.current = {
                              ...saved.current,
                              destination: null,
                            };
                        })
                      }
                    >
                      {t("Clear")}
                    </Button>
                  </div>
                </Field>
                <Field
                  label={t("Language")}
                  hint={t(
                    "Language changes apply immediately. Save settings to remember your choice.",
                  )}
                >
                  <Select
                    value={settings.language}
                    onChange={(_, d) =>
                      patch({ language: d.value as Language })
                    }
                  >
                    <option value="system">{t("System")}</option>
                    <option value="en-US" lang="en-US">
                      English
                    </option>
                    <option value="zh-CN" lang="zh-CN">
                      简体中文
                    </option>
                  </Select>
                </Field>
                <Field label={t("Appearance")}>
                  <Select
                    value={settings.theme}
                    onChange={(_, d) => patch({ theme: d.value })}
                  >
                    <option value="system">{t("System")}</option>
                    <option value="light">{t("Light")}</option>
                    <option value="dark">{t("Dark")}</option>
                  </Select>
                </Field>
                <Field label={t("Log level")}>
                  <Select
                    value={settings.verbose}
                    onChange={(_, d) => patch({ verbose: d.value })}
                  >
                    {["none", "error", "warning", "info", "debug"].map((v) => (
                      <option key={v} value={v}>
                        {v === "none"
                          ? t("None")
                          : v === "error"
                            ? t("Error")
                            : v === "warning"
                              ? t("Warning")
                              : v === "info"
                                ? t("Info")
                                : t("Debug")}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Checkbox
                  label={t("Reduce status notifications")}
                  checked={settings.quiet}
                  onChange={(_, d) => patch({ quiet: d.checked === true })}
                />
                <Button
                  appearance="primary"
                  disabled={busy || settings.server !== info?.settings.server}
                  onClick={() =>
                    void work(async () => {
                      const updated = { ...settings, version: 2 };
                      await api.save(updated);
                      setSettings(updated);
                      saved.current = updated;
                      setNotice({
                        text: "Settings saved for the next transfer.",
                        error: false,
                      });
                    })
                  }
                >
                  {t("Save settings")}
                </Button>
              </Card>
              <Card className="panel">
                <Title2>{t("Server & trust")}</Title2>
                <Field label={t("Signaling server")}>
                  <Input
                    value={settings.server}
                    onChange={(_, d) => {
                      patch({ server: d.value });
                      setFingerprint("");
                    }}
                  />
                </Field>
                <p className="subtle">
                  {t(
                    "Only one server is trusted. Verifying a new server replaces the current trust and switches the server.",
                  )}
                </p>
                <Field label={t("Expected SPKI fingerprint")}>
                  <Textarea
                    value={fingerprint}
                    onChange={(_, d) => setFingerprint(d.value)}
                    resize="vertical"
                  />
                </Field>
                <div className="actions">
                  <Button
                    disabled={busy}
                    onClick={() =>
                      void work(async () =>
                        setFingerprint(await api.probe(settings.server)),
                      )
                    }
                  >
                    {t("Fetch fingerprint")}
                  </Button>
                  <Button
                    disabled={busy || !/^[a-fA-F0-9]{64}$/.test(fingerprint)}
                    onClick={() => setTrusting(true)}
                  >
                    {t("Verify & trust")}
                  </Button>
                </div>
                <Title2>{t("Connection details")}</Title2>
                <dl>
                  <dt>{t("Local IPv4 addresses")}</dt>
                  <dd>
                    {info?.ipv4?.join("\n") || t("No local IPv4 address found")}
                  </dd>
                  <dt>{t("Local IPv6 addresses")}</dt>
                  <dd>
                    {info?.ipv6.join("\n") || t("No usable IPv6 address found")}
                  </dd>
                  <dt>{t("Version")}</dt>
                  <dd>{info?.version}</dd>
                  <dt>{t("Trusted server")}</dt>
                  <dd>{Object.keys(info?.trusted_servers ?? {}).join("\n")}</dd>
                </dl>
              </Card>
            </div>
          )}
          {page === "logs" && (
            <Card className="panel">
              <div className="toolbar">
                <p>
                  {t(
                    "Log level follows your saved settings. Logs are bounded and kept in memory.",
                  )}
                </p>
                <Button
                  onClick={() =>
                    void work(async () => setLogs(await api.logs()))
                  }
                >
                  {t("Refresh logs")}
                </Button>
              </div>
              <pre aria-label={t("Connection logs")}>
                {logs.join("\n") ||
                  t("No logs. Select a log level in Settings.")}
              </pre>
            </Card>
          )}
        </MotionRegion>
      </main>
      <Dialog
        open={!!removing}
        onOpenChange={(_, d) => {
          if (!d.open) setRemoving(null);
        }}
      >
        <DialogSurface>
          <DialogBody>
            <DialogTitle>{t("Remove this inbox copy?")}</DialogTitle>
            <DialogContent>
              {removing?.name}
              <p>{t("Copies already exported will stay on your device.")}</p>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setRemoving(null)}>
                {t("Keep file")}
              </Button>
              <Button
                appearance="primary"
                disabled={busy || running}
                onClick={() =>
                  void work(async () => {
                    await api.remove(removing!.id);
                    setRemoving(null);
                    const received = await api.files();
                    transition(() => setFiles(received));
                  })
                }
              >
                {t("Remove copy")}
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <Dialog open={trusting} onOpenChange={(_, d) => setTrusting(d.open)}>
        <DialogSurface>
          <DialogBody>
            <DialogTitle>{t("Trust this server?")}</DialogTitle>
            <DialogContent>
              <p>
                {t(
                  "Compare this fingerprint with a trusted source. Fetching it alone does not establish the server's identity.",
                )}
              </p>
              <p>{settings?.server}</p>
              <code className="fingerprint">{fingerprint}</code>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setTrusting(false)}>{t("Cancel")}</Button>
              <Button
                appearance="primary"
                disabled={busy}
                onClick={() =>
                  void work(async () => {
                    await api.trust(settings!.server, fingerprint);
                    setTrusting(false);
                    const updated = await api.bootstrap();
                    setInfo(updated);
                    patch({ server: updated.settings.server });
                    if (saved.current)
                      saved.current = {
                        ...saved.current,
                        server: updated.settings.server,
                      };
                    report(
                      "Server fingerprint verified. The previous trust was replaced and the new server is active.",
                    );
                  })
                }
              >
                {t("Trust verified fingerprint")}
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </UiProvider>
  );
}
createRoot(document.getElementById("app")!).render(<App />);
