import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests",
  fullyParallel: true,
  use: { baseURL: "http://127.0.0.1:1420", headless: true },
  webServer: {
    command: "npm run dev",
    url: "http://127.0.0.1:1420",
    reuseExistingServer: !process.env.CI,
  },
  projects: [
    { name: "desktop", use: { viewport: { width: 1280, height: 900 } } },
    {
      name: "android-phone",
      use: {
        viewport: { width: 390, height: 844 },
        isMobile: true,
        hasTouch: true,
      },
    },
    {
      name: "android-tablet",
      use: {
        viewport: { width: 1100, height: 900 },
        isMobile: true,
        hasTouch: true,
      },
    },
  ],
});
