import React, { useEffect, useRef, useState } from "react";
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
  ProgressBar,
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
import "./style.css";
const size = (n: number | null) =>
  n === null
    ? "Unknown size"
    : n < 1024
      ? `${n} B`
      : n < 1048576
        ? `${(n / 1024).toFixed(1)} KiB`
        : n < 1073741824
          ? `${(n / 1048576).toFixed(1)} MiB`
          : `${(n / 1073741824).toFixed(2)} GiB`;
function App() {
  const [info, setInfo] = useState<Bootstrap | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [page, setPage] = useState("transfer");
  const [mode, setMode] = useState("file");
  const [source, setSource] = useState<Selection | null>(null);
  const [code, setCode] = useState("");
  const [text, setText] = useState("");
  const [status, setStatus] = useState<Status | null>(null);
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
        setStatus(value);
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
      setStatus(await api.status());
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
          aria-label="Main navigation"
          selectedValue={page}
          onTabSelect={(_, d) => setPage(String(d.value))}
        >
          <Tab value="transfer" icon={<ArrowUpload24Regular />}>
            Transfer
          </Tab>
          <Tab value="inbox" icon={<ArrowDownload24Regular />}>
            Inbox
          </Tab>
          <Tab value="settings" icon={<Settings24Regular />}>
            Settings
          </Tab>
          <Tab value="logs" icon={<Document24Regular />}>
            Logs
          </Tab>
        </TabList>
        <div className="rail-footer">
          <ShieldCheckmark24Regular />
          <span>
            Encrypted peer to peer
            <br />
            IPv6 · No file relay
          </span>
        </div>
      </aside>
      <main>
        <header>
          <div>
            <Title1>
              {page === "transfer"
                ? "A direct connection."
                : page === "inbox"
                  ? "Your verified files."
                  : page === "settings"
                    ? "Make it yours."
                    : "Connection logs."}
            </Title1>
            <p className="subtle">
              {page === "transfer"
                ? "Send a file or a message. Share the code. Stay in control."
                : "Hermod · private file transfer"}
            </p>
          </div>
          <Badge
            appearance="tint"
            color={info?.ipv6.length ? "success" : "warning"}
          >
            {info?.ipv6.length ? "IPv6 available" : "IPv6 required"}
          </Badge>
        </header>
        {notice && (
          <MessageBar intent={notice.error ? "error" : "success"}>
            <MessageBarBody>{notice.text}</MessageBarBody>
            <Button
              appearance="transparent"
              aria-label="Dismiss message"
              onClick={() => setNotice(null)}
            >
              Dismiss
            </Button>
          </MessageBar>
        )}
        {!settings && <Spinner label="Loading settings" />}
        {settings && page === "transfer" && (
          <div className="transfer-grid">
            <Card className="panel">
              <TabList
                aria-label="Transfer type"
                selectedValue={mode}
                onTabSelect={(_, d) => setMode(String(d.value))}
                disabled={running || busy}
              >
                <Tab value="file">File</Tab>
                <Tab value="text">Text</Tab>
                <Tab value="receive">Receive</Tab>
              </TabList>
              {mode === "file" && (
                <>
                  <Title2>Choose what to send</Title2>
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
                    <strong>{source?.name ?? "Choose file"}</strong>
                    <span>
                      {source
                        ? size(source.size)
                        : "Open the system file picker"}
                    </span>
                  </button>
                </>
              )}
              {mode === "text" && (
                <Field
                  label="Message"
                  hint="Up to 1 MiB; received as a verified text file"
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
                  <Title2>Connect with a code</Title2>
                  <Field
                    label="Transfer code"
                    hint="Ask your sender for their complete code"
                  >
                    <Input
                      value={code}
                      onChange={(_, d) => setCode(d.value)}
                      disabled={running || busy}
                      placeholder="1234-word-word-word"
                    />
                  </Field>
                  <p>
                    Verified files stay in your private inbox. Save a copy
                    anywhere.
                  </p>
                </>
              )}
              <div className="summary">
                <span>
                  {mode === "receive"
                    ? saved.current?.receive_transport
                    : saved.current?.connection.transport}{" "}
                  ·{" "}
                  {saved.current?.connection.limit_mbps === "0"
                    ? "Unlimited"
                    : `${saved.current?.connection.limit_mbps} Mbps cap`}
                </span>
                <Button appearance="subtle" onClick={() => setPage("settings")}>
                  Adjust
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
                  ? "Connect & receive"
                  : "Create transfer code"}
              </Button>
              <p className="subtle">
                Both devices need reachable IPv6. Keep the transfer code
                private.
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
                {running ? "ACTIVE" : status?.digest ? "VERIFIED" : "READY"}
              </Badge>
              <Title2>{status?.phase ?? "Ready when you are."}</Title2>
              {!status && (
                <p className="subtle">
                  Your file travels directly between devices. The signaling
                  server only helps them meet.
                </p>
              )}
              {status?.export_error && (
                <MessageBar intent="error">
                  <MessageBarBody>{status.export_error}</MessageBarBody>
                </MessageBar>
              )}
              {status?.error && (
                <MessageBar intent="error">
                  <MessageBarBody>{status.error}</MessageBarBody>
                </MessageBar>
              )}
              {status?.code && (
                <div className="code-box">
                  <span>Share this code</span>
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
                    Copy code
                  </Button>
                </div>
              )}
              {status && (
                <>
                  <ProgressBar
                    aria-label="Transfer progress"
                    value={
                      status.total
                        ? Math.min(1, status.bytes / status.total)
                        : undefined
                    }
                  />
                  <div className="summary">
                    <span>
                      {size(status.bytes)} / {size(status.total)}
                    </span>
                    <span>{status.transport ?? "Connecting"}</span>
                  </div>
                  <div className="metrics">
                    <div>
                      <strong>
                        {size(
                          status.elapsed > 0
                            ? status.bytes / status.elapsed
                            : 0,
                        )}
                        /s
                      </strong>
                      <span>Average payload</span>
                    </div>
                    <div>
                      <strong>
                        {status.rtt_ms == null
                          ? "—"
                          : `${status.rtt_ms.toFixed(0)} ms`}
                      </strong>
                      <span>Network RTT</span>
                    </div>
                  </div>
                </>
              )}
              {status?.digest && (
                <div className="digest">
                  <ShieldCheckmark24Regular />
                  <span>SHA-256 verified</span>
                  <code>{status.digest}</code>
                </div>
              )}
              {running && (
                <Button
                  onClick={() =>
                    void work(async () => {
                      await api.cancel();
                      setStatus(await api.status());
                    })
                  }
                >
                  Cancel transfer
                </Button>
              )}
            </Card>
          </div>
        )}
        {page === "inbox" && (
          <>
            <div className="toolbar">
              <span>{files.length} verified files</span>
              <Button
                onClick={() =>
                  void work(async () => setFiles(await api.files()))
                }
              >
                Refresh
              </Button>
            </div>
            {files.length === 0 && (
              <Card className="panel">
                <Title2>Your inbox is empty</Title2>
                <p>Completed, verified transfers will appear here.</p>
              </Card>
            )}
            {files.map((file) => (
              <Card key={file.id} className="inbox-row">
                <Document24Regular />
                <div className="file-detail">
                  <strong>{file.name}</strong>
                  <span className="subtle">
                    {size(file.size)} · SHA-256 verified
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
                  Save a copy
                </Button>
                <Button
                  disabled={busy || running}
                  icon={<Delete24Regular />}
                  aria-label={`Remove ${file.name}`}
                  onClick={() => setRemoving(file)}
                />
              </Card>
            ))}
            <p className="subtle">
              Export important files before uninstalling. Removing an inbox copy
              does not remove exported copies.
            </p>
          </>
        )}
        {settings && page === "settings" && (
          <div className="settings-grid">
            <Card className="panel">
              <Title2>Transfer preferences</Title2>
              <Field label="Send protocol">
                <Select
                  value={settings.connection.transport}
                  onChange={(_, d) =>
                    network({ transport: d.value, congestion: "auto" })
                  }
                >
                  <option value="wireguard">WireGuard (default)</option>
                  <option value="quic">QUIC</option>
                </Select>
              </Field>
              <Field label="Receive protocol">
                <Select
                  value={settings.receive_transport}
                  onChange={(_, d) => patch({ receive_transport: d.value })}
                >
                  <option value="auto">Auto — follow the sender</option>
                  <option value="wireguard">WireGuard only</option>
                  <option value="quic">QUIC only</option>
                </Select>
              </Field>
              <Field
                label="Payload limit (Mbps)"
                hint="0 = unlimited. Start at 70–85% of your upload speed."
              >
                <Input
                  value={settings.connection.limit_mbps}
                  type="number"
                  min={0}
                  step="0.1"
                  onChange={(_, d) => network({ limit_mbps: d.value })}
                />
              </Field>
              <Field label="Code length (words)">
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
                  <AccordionHeader>Advanced network settings</AccordionHeader>
                  <AccordionPanel className="field-stack">
                    <Field
                      label="MTU"
                      hint="auto, or bytes: WireGuard inner IP; QUIC UDP payload cap (1200–1452)."
                    >
                      <Input
                        value={settings.connection.mtu}
                        onChange={(_, d) => network({ mtu: d.value })}
                      />
                    </Field>
                    <Field label="Congestion control">
                      <Select
                        value={settings.connection.congestion}
                        onChange={(_, d) => network({ congestion: d.value })}
                      >
                        {algorithms.map((a) => (
                          <option key={a} value={a}>
                            {a}
                          </option>
                        ))}
                      </Select>
                    </Field>
                  </AccordionPanel>
                </AccordionItem>
              </Accordion>
              <Field
                label="Default export folder"
                hint="A verified inbox copy is always retained."
              >
                <div className="actions">
                  <Input
                    readOnly
                    value={settings.destination ?? "Private inbox only"}
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
                    Choose folder
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
                    Clear
                  </Button>
                </div>
              </Field>
              <Field label="Appearance">
                <Select
                  value={settings.theme}
                  onChange={(_, d) => patch({ theme: d.value })}
                >
                  <option value="system">System</option>
                  <option value="light">Light</option>
                  <option value="dark">Dark</option>
                </Select>
              </Field>
              <Field label="Log level">
                <Select
                  value={settings.verbose}
                  onChange={(_, d) => patch({ verbose: d.value })}
                >
                  {["none", "error", "warning", "info", "debug"].map((v) => (
                    <option key={v}>{v}</option>
                  ))}
                </Select>
              </Field>
              <Checkbox
                label="Reduce status notifications"
                checked={settings.quiet}
                onChange={(_, d) => patch({ quiet: d.checked === true })}
              />
              <Button
                appearance="primary"
                disabled={busy}
                onClick={() =>
                  void work(async () => {
                    await api.save(settings);
                    saved.current = settings;
                    setNotice({
                      text: "Settings saved for the next transfer.",
                      error: false,
                    });
                  })
                }
              >
                Save settings
              </Button>
            </Card>
            <Card className="panel">
              <Title2>Server & trust</Title2>
              <Field label="Signaling server">
                <Input
                  value={settings.server}
                  onChange={(_, d) => {
                    patch({ server: d.value });
                    setFingerprint("");
                  }}
                />
              </Field>
              <p className="subtle">
                The default server is pinned. For another server, verify its
                fingerprint before trusting it.
              </p>
              <Field label="Expected SPKI fingerprint">
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
                  Fetch fingerprint
                </Button>
                <Button
                  disabled={busy || !/^[a-fA-F0-9]{64}$/.test(fingerprint)}
                  onClick={() => setTrusting(true)}
                >
                  Verify & trust
                </Button>
              </div>
              <Title2>Connection details</Title2>
              <dl>
                <dt>Local IPv6 addresses</dt>
                <dd>
                  {info?.ipv6.join("\n") || "No usable IPv6 address found"}
                </dd>
                <dt>Version</dt>
                <dd>{info?.version}</dd>
                <dt>Trusted servers</dt>
                <dd>{Object.keys(info?.trusted_servers ?? {}).join("\n")}</dd>
              </dl>
            </Card>
          </div>
        )}
        {page === "logs" && (
          <Card className="panel">
            <div className="toolbar">
              <p>
                Log level follows your saved settings. Logs are bounded and kept
                in memory.
              </p>
              <Button
                onClick={() => void work(async () => setLogs(await api.logs()))}
              >
                Refresh logs
              </Button>
            </div>
            <pre aria-label="Connection logs">
              {logs.join("\n") || "No logs. Select a log level in Settings."}
            </pre>
          </Card>
        )}
      </main>
      <Dialog
        open={!!removing}
        onOpenChange={(_, d) => {
          if (!d.open) setRemoving(null);
        }}
      >
        <DialogSurface>
          <DialogBody>
            <DialogTitle>Remove this inbox copy?</DialogTitle>
            <DialogContent>
              {removing?.name}
              <p>Copies already exported will stay on your device.</p>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setRemoving(null)}>Keep file</Button>
              <Button
                appearance="primary"
                disabled={busy || running}
                onClick={() =>
                  void work(async () => {
                    await api.remove(removing!.id);
                    setRemoving(null);
                    setFiles(await api.files());
                  })
                }
              >
                Remove copy
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
      <Dialog open={trusting} onOpenChange={(_, d) => setTrusting(d.open)}>
        <DialogSurface>
          <DialogBody>
            <DialogTitle>Trust this server?</DialogTitle>
            <DialogContent>
              <p>
                Compare this fingerprint with a trusted source. Fetching it
                alone does not establish the server's identity.
              </p>
              <p>{settings?.server}</p>
              <code className="fingerprint">{fingerprint}</code>
            </DialogContent>
            <DialogActions>
              <Button onClick={() => setTrusting(false)}>Cancel</Button>
              <Button
                appearance="primary"
                disabled={busy}
                onClick={() =>
                  void work(async () => {
                    await api.trust(settings!.server, fingerprint);
                    setTrusting(false);
                    setInfo(await api.bootstrap());
                    report(
                      "Server fingerprint verified and pinned. Save settings to use this server.",
                    );
                  })
                }
              >
                Trust verified fingerprint
              </Button>
            </DialogActions>
          </DialogBody>
        </DialogSurface>
      </Dialog>
    </UiProvider>
  );
}
createRoot(document.getElementById("app")!).render(<App />);
