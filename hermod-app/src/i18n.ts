import chinese from "./locales/zh-CN.json" with { type: "json" };

export type Locale = "en-US" | "zh-CN";
export type Language = "system" | Locale;
export type Message = keyof typeof chinese;

export function resolveLocale(
  language: string,
  preferred: readonly string[],
): Locale {
  if (language === "zh-CN" || language === "en-US") return language;
  for (const locale of preferred) {
    if (/^zh(?:-|$)/i.test(locale)) return "zh-CN";
    if (/^en(?:-|$)/i.test(locale)) return "en-US";
  }
  return "en-US";
}

export function translator(locale: Locale) {
  return (message: Message, values: Record<string, string | number> = {}) => {
    const text = locale === "zh-CN" ? chinese[message] : message;
    return text.replace(/\{(\w+)\}/g, (token, name: string) =>
      String(values[name] ?? token),
    );
  };
}

// 未知系统错误保留原文以便排查；已知状态和错误与 Rust 原生对话框共用词典。
export function runtimeText(
  text: string,
  locale: Locale,
  error = false,
): string {
  const message = text.replace(/^Error: /, "");
  if (Object.hasOwn(chinese, message))
    return translator(locale)(message as Message);
  return error && locale === "zh-CN"
    ? `${translator(locale)("Operation failed. Technical details:")}\n${text}`
    : text;
}
