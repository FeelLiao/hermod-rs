import { test, expect } from "@playwright/test";
test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    const defaults = {
      connection: {
        transport: "wireguard",
        mtu: "auto",
        congestion: "auto",
        limit_mbps: "8",
      },
      receive_transport: "auto",
      server: "wss://relay.example:4376",
      words: 3,
      verbose: "none",
      quiet: false,
      theme: "system",
      destination: null,
    };
    let settings = JSON.parse(
      localStorage.getItem("mock-settings") ?? JSON.stringify(defaults),
    );
    let status: any = null;
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
          case "bootstrap":
            return {
              version: "0.4.1",
              ipv6: ["2001:db8::1"],
              platform: "linux",
              settings: structuredClone(settings),
              settings_saved: true,
              trusted_servers: { "wss://relay.example:4376": "a".repeat(64) },
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
  await expect(page.getByLabel("Send protocol", { exact: true })).toHaveValue("quic");
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
});
