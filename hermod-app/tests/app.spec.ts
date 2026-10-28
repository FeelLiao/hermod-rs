import { test, expect } from "@playwright/test";
import { msixVersion } from "../scripts/msix-version.mjs";
import { resolveLocale, runtimeText } from "../src/i18n";
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const defaults = {
      version: 2,
      connection: {
        transport: "quic",
        mtu: "auto",
        congestion: "auto",
        limit_mbps: "0",
      },
      receive_transport: "auto",
      server: "wss://relay.example:4376",
      words: 3,
      verbose: "none",
      quiet: false,
      theme: "system",
      language: "system",
      destination: null,
    };
    let settings = JSON.parse(
      localStorage.getItem("mock-settings") ?? JSON.stringify(defaults),
    );
    let status: any = null;
    let trusted = { [settings.server]: "a".repeat(64) };
    (window as any).setMockStatus = (value: any) => {
      status = value;
    };
    const calls: any[] = [];
    const files = [
      {
        id: "a".repeat(32),
        name: "地球 3.0.zip",
        size: 10485760,
        sha256: "f".repeat(64),
      },
    ];
    (window as any).calls = calls;
    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (command: string, args: any) => {
        calls.push({ command, args });
        switch (command) {
          case "set_locale":
            return;
          case "bootstrap":
            return {
              version: "0.4.3",
              ipv6: ["2001:db8::1"],
              ipv4: ["192.168.1.2"],
              platform: "linux",
              settings: structuredClone(settings),
              settings_saved: true,
              trusted_servers: structuredClone(trusted),
            };
          case "save_settings":
            settings = structuredClone(args.settings);
            localStorage.setItem("mock-settings", JSON.stringify(settings));
            return;
          case "status":
            return status;
          case "choose_source":
            return { id: "source", name: "地球 3.0.zip", size: 10485760 };
          case "start_transfer":
            status = {
              phase: "Waiting for the receiver",
              code: args.code ? null : "1234-word-word-word",
              bytes: 0,
              total: 10485760,
              elapsed: 0,
              done: false,
              cancelled: false,
              error: null,
              digest: null,
              rtt_ms: null,
              transport: null,
              received_id: null,
            };
            return;
          case "cancel_transfer":
            status = {
              ...status,
              done: true,
              cancelled: true,
              phase: "Transfer cancelled",
            };
            return;
          case "copy_code":
            return;
          case "list_received":
            return files;
          case "export_received":
            return true;
          case "delete_received":
            files.length = 0;
            return;
          case "get_logs":
            return ["INFO transport established"];
          case "probe_server":
            return "b".repeat(64);
          case "trust_server":
            settings.server = args.server;
            trusted = { [args.server]: args.fingerprint };
            return;
          case "choose_destination":
            settings.destination = "/exports";
            return "/exports";
          case "clear_destination":
            settings.destination = null;
            return;
          default:
            throw Error(`Unexpected command ${command}`);
        }
      },
    };
  });
});
test("desktop file selection, transfer code and cancellation work", async ({
  page,
}, info) => {
  await page.goto("/");
  await expect(
    page.getByRole("button", { name: "Create transfer code" }),
  ).toBeDisabled();
  await page.getByRole("button", { name: /Choose file/ }).click();
  await expect(page.getByText("地球 3.0.zip", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Create transfer code" }).click();
  await expect(page.getByTestId("transfer-code")).toHaveText(
    "1234-word-word-word",
  );
  await page.getByRole("button", { name: "Copy code" }).click();
  await expect(page.getByText("Transfer code copied.")).toBeVisible();
  await page.getByRole("button", { name: "Cancel transfer" }).click();
  await expect(
    page.getByText("Transfer cancelled", { exact: true }),
  ).toBeVisible();
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page.screenshot({
    path: `test-results/${info.project.name}-transfer.png`,
    fullPage: true,
  });
});
test("shared network settings persist and text input replaces stdin", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await page.getByLabel("Send protocol", { exact: true }).selectOption("quic");
  await page.getByRole("button", { name: "Advanced network settings" }).click();
  await page.getByLabel("MTU", { exact: true }).fill("1280");
  await page.getByLabel("Congestion control").selectOption("bbr");
  await page.getByLabel("Payload limit (Mbps)").fill("6.5");
  await page.getByRole("button", { name: "Save settings" }).click();
  await expect(
    page.getByText("Settings saved for the next transfer."),
  ).toBeVisible();
  await page.reload();
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await expect(page.getByLabel("Send protocol", { exact: true })).toHaveValue(
    "quic",
  );
  await expect(page.getByLabel("Payload limit (Mbps)")).toHaveValue("6.5");
  await page.getByRole("tab", { name: "Transfer", exact: true }).click();
  await page.getByRole("tab", { name: "Text", exact: true }).click();
  await page.getByLabel("Message", { exact: true }).fill("Hello 世界");
  await page.getByRole("button", { name: "Create transfer code" }).click();
  expect(
    await page.evaluate(
      () =>
        (window as any).calls.find((c: any) => c.command === "start_transfer")
          .args,
    ),
  ).toEqual({ sourceId: null, code: null, text: "Hello 世界" });
});
test("receive validates input and inbox export never exposes a source path", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "Receive", exact: true }).click();
  await page.getByLabel("Transfer code", { exact: true }).fill("bad");
  await page.getByRole("button", { name: "Connect & receive" }).click();
  await expect(page.getByText(/Enter a valid transfer code/)).toBeVisible();
  await page.getByRole("tab", { name: "Inbox", exact: true }).click();
  await expect(page.getByText("地球 3.0.zip")).toBeVisible();
  await page.getByRole("button", { name: "Save a copy" }).click();
  expect(
    await page.evaluate(
      () =>
        (window as any).calls.find((c: any) => c.command === "export_received")
          .args,
    ),
  ).toEqual({ id: "a".repeat(32), useDefault: false });
  await page.getByRole("button", { name: "Remove 地球 3.0.zip" }).click();
  await page.getByRole("button", { name: "Keep file" }).click();
  await expect(page.getByText("地球 3.0.zip")).toBeVisible();
});
test("server trust requires explicit fingerprint confirmation", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await page
    .getByLabel("Signaling server", { exact: true })
    .fill("wss://private.example");
  await page.getByRole("button", { name: "Fetch fingerprint" }).click();
  await expect(page.getByLabel("Expected SPKI fingerprint")).toHaveValue(
    "b".repeat(64),
  );
  expect(
    await page.evaluate(() =>
      (window as any).calls.some((c: any) => c.command === "trust_server"),
    ),
  ).toBe(false);
  await page.getByRole("button", { name: "Verify & trust" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page
    .getByRole("button", { name: "Trust verified fingerprint" })
    .click();
  await expect(page.getByRole("dialog")).not.toBeVisible();
  expect(
    await page.evaluate(
      () =>
        (window as any).calls.find((c: any) => c.command === "trust_server")
          .args.fingerprint,
    ),
  ).toBe("b".repeat(64));
  await expect(
    page.getByText(
      "Server fingerprint verified. The previous trust was replaced and the new server is active.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Save settings" }),
  ).toBeEnabled();
  await expect(
    page.locator("dd").filter({ hasText: "wss://relay.example:4376" }),
  ).toHaveCount(0);
});

test("new defaults and explicit legacy preference migration persist", async ({
  page,
}) => {
  await page.goto("/");
  await expect(
    page.getByText("QUIC · Unlimited", { exact: true }),
  ).toBeVisible();
  await page.evaluate(() =>
    localStorage.setItem(
      "mock-settings",
      JSON.stringify({
        version: 1,
        connection: {
          transport: "wireguard",
          limit_mbps: "8",
          mtu: "auto",
          congestion: "auto",
        },
        receive_transport: "auto",
        server: "wss://relay.example:4376",
        words: 3,
        verbose: "none",
        quiet: false,
        theme: "system",
        language: "en-US",
        destination: null,
      }),
    ),
  );
  await page.reload();
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await expect(page.getByLabel("Send protocol", { exact: true })).toHaveValue(
    "wireguard",
  );
  await expect(page.getByLabel("Payload limit (Mbps)")).toHaveValue("8");
  await page.getByRole("button", { name: "Use new defaults" }).click();
  await page.getByRole("button", { name: "Save settings" }).click();
  await page.reload();
  await expect(
    page.getByText("QUIC · Unlimited", { exact: true }),
  ).toBeVisible();
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Use new defaults" }),
  ).toHaveCount(0);
});

test("receive shows announced totals before bytes and handles empty files", async ({
  page,
}) => {
  await page.goto("/");
  const status = {
    phase: "Receiving file",
    code: null,
    bytes: 0,
    total: 1048576,
    elapsed: 0,
    done: false,
    cancelled: false,
    error: null,
    digest: null,
    rtt_ms: null,
    transport: "quic",
    received_id: null,
  };
  await page.evaluate(
    (status) => (window as any).setMockStatus(status),
    status,
  );
  await expect(page.getByText("0 B / 1.0 MiB")).toBeVisible();
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "0",
  );
  await page.evaluate(
    (status) => (window as any).setMockStatus({ ...status, total: 0 }),
    status,
  );
  await expect(page.getByText("0 B / 0 B")).toBeVisible();
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "0",
  );
  await page.evaluate(
    (status) =>
      (window as any).setMockStatus({
        ...status,
        total: 0,
        done: true,
        digest: "a".repeat(64),
      }),
    status,
  );
  await expect(page.getByRole("progressbar")).toHaveAttribute(
    "aria-valuenow",
    "1",
  );
});

test("page motion runs and reduced motion disables page transitions", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await page.goto("/");
  await page.getByRole("button", { name: "Create transfer code" }).waitFor();
  await page.evaluate(() => {
    const original = document.startViewTransition.bind(document);
    (window as any).transitionCount = 0;
    document.startViewTransition = ((update: any) => {
      (window as any).transitionCount++;
      const transition = original(update);
      void transition.ready.then(() => {
        (window as any).animatedPage = document
          .getAnimations()
          .some((animation) =>
            ["page-in", "page-out"].includes(
              (animation as CSSAnimation).animationName,
            ),
          );
      });
      return transition;
    }) as typeof document.startViewTransition;
  });
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await expect
    .poll(() => page.evaluate(() => (window as any).animatedPage))
    .toBe(true);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.getByRole("tab", { name: "Transfer", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Create transfer code" }),
  ).toBeVisible();
  expect(await page.evaluate(() => (window as any).transitionCount)).toBe(1);
});

test("language choice updates the UI and native dialogs and persists", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "Settings", exact: true }).click();
  await page.getByLabel("Language", { exact: true }).selectOption("zh-CN");
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await expect(page.getByLabel("发送协议", { exact: true })).toHaveValue(
    "quic",
  );
  await page.getByRole("button", { name: "保存设置" }).click();
  await expect(
    page.getByText("设置已保存，传输设置将在下次传输时生效。"),
  ).toBeVisible();
  await page.reload();
  await page.getByRole("tab", { name: "设置", exact: true }).click();
  await expect(page.getByLabel("语言", { exact: true })).toHaveValue("zh-CN");
  expect(
    await page.evaluate(
      () =>
        (window as any).calls
          .filter((c: any) => c.command === "set_locale")
          .at(-1).args.locale,
    ),
  ).toBe("zh-CN");
  await page.getByRole("tab", { name: "收件箱", exact: true }).click();
  await page.getByRole("button", { name: "另存副本" }).click();
  await expect(page.getByText("已保存经过验证的副本。")).toBeVisible();
  await page.getByRole("button", { name: "删除 地球 3.0.zip" }).click();
  await expect(page.getByRole("dialog")).toContainText("删除此收件箱副本？");
  await page.getByRole("button", { name: "保留文件" }).click();
  await page.getByRole("tab", { name: "设置", exact: true }).click();
  await page.getByLabel("语言", { exact: true }).selectOption("en-US");
  await page.getByRole("button", { name: "Save settings" }).click();
  await page.reload();
  await expect(
    page.getByRole("tab", { name: "Transfer", exact: true }),
  ).toBeVisible();
});

test.describe("Chinese system language", () => {
  test.use({ locale: "zh-CN" });
  test("new installations follow system language for validation and progress", async ({
    page,
  }) => {
    await page.goto("/");
    await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
    await page.getByRole("tab", { name: "接收", exact: true }).click();
    await page.getByLabel("传输码", { exact: true }).fill("bad");
    await page.getByRole("button", { name: "连接并接收" }).click();
    await expect(page.getByText("请输入有效的传输码。")).toBeVisible();
    await page.getByRole("tab", { name: "文本", exact: true }).click();
    await page.getByLabel("文本内容", { exact: true }).fill("你好，Hermod！");
    await page.getByRole("button", { name: "生成传输码" }).click();
    await expect(
      page.getByText("等待接收方连接", { exact: true }),
    ).toBeVisible();
    await page.getByRole("button", { name: "复制传输码" }).click();
    await expect(page.getByText("传输码已复制。")).toBeVisible();
    await page.getByRole("button", { name: "取消传输" }).click();
    await expect(page.getByText("传输已取消", { exact: true })).toBeVisible();
    await page.screenshot({
      path: "test-results/desktop-zh-CN.png",
      fullPage: true,
    });
  });
});

test("locale fallback and Store version mapping handle boundaries", () => {
  expect(resolveLocale("system", ["fr-FR", "zh-Hans-CN"])).toBe("zh-CN");
  expect(resolveLocale("system", ["en-GB", "zh-CN"])).toBe("en-US");
  expect(resolveLocale("en-US", ["zh-CN"])).toBe("en-US");
  expect(resolveLocale("system", ["fr-FR"])).toBe("en-US");
  expect(
    runtimeText("Error: Enter a valid transfer code.", "zh-CN", true),
  ).toBe("请输入有效的传输码。");
  expect(runtimeText("provider-specific diagnostic", "zh-CN", true)).toContain(
    "provider-specific diagnostic",
  );
  expect(msixVersion("0.4.3")).toBe("1.4.3.0");
  expect(msixVersion("1.0.0")).toBe("2.0.0.0");
  expect(msixVersion("65534.65535.65535")).toBe("65535.65535.65535.0");
  for (const invalid of [
    "65535.0.0",
    "0.65536.0",
    "0.4.3-beta",
    "0.4",
    "01.2.3",
  ]) {
    expect(() => msixVersion(invalid)).toThrow();
  }
});
