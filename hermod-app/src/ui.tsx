// 两个平台共用页面；这个适配层只负责官方控件和事件签名的差异。
import React, {
  createContext,
  useContext,
  useEffect,
  useId,
  useState,
} from "react";
import * as F from "@fluentui/react-components";
import "@material/web/button/filled-button.js";
import "@material/web/button/outlined-button.js";
import "@material/web/button/text-button.js";
import "@material/web/textfield/outlined-text-field.js";
import "@material/web/select/outlined-select.js";
import "@material/web/select/select-option.js";
import "@material/web/checkbox/checkbox.js";
import "@material/web/progress/linear-progress.js";
import "@material/web/progress/circular-progress.js";
import "@material/web/dialog/dialog.js";
import "@material/web/tabs/tabs.js";
import "@material/web/tabs/primary-tab.js";
import "@material/web/ripple/ripple.js";
import "@material/web/focus/md-focus-ring.js";
import {
  argbFromHex,
  themeFromSourceColor,
  applyTheme,
} from "@material/material-color-utilities";
import "./material.css";
import { reducedMotion } from "./motion";

const Platform = createContext(false);
const FieldLabel = createContext("");
const AccordionState = createContext({ open: false, id: "", toggle: () => {} });
function useReducedMotion() {
  const [reduce, setReduce] = useState(reducedMotion);
  useEffect(() => {
    const media = matchMedia("(prefers-reduced-motion: reduce)");
    const changed = () => setReduce(media.matches);
    media.addEventListener("change", changed);
    return () => media.removeEventListener("change", changed);
  }, []);
  return reduce;
}
const h = React.createElement;
export const webDarkTheme = F.webDarkTheme;
export const webLightTheme = F.webLightTheme;
export function UiProvider({
  android,
  seed,
  dark,
  children,
  className,
}: {
  android: boolean;
  seed?: string;
  dark: boolean;
  children: React.ReactNode;
  className: string;
}) {
  useEffect(() => {
    document.documentElement.dataset.ui = android ? "material" : "fluent";
    if (android)
      applyTheme(themeFromSourceColor(argbFromHex(seed ?? "#385ca8")), {
        target: document.documentElement,
        dark,
      });
    document.documentElement.style.colorScheme = dark ? "dark" : "light";
  }, [android, seed, dark]);
  return (
    <Platform.Provider value={android}>
      {android ? (
        <div
          className={`${className} material`}
          data-theme={dark ? "dark" : "light"}
        >
          {children}
        </div>
      ) : (
        <F.FluentProvider
          theme={dark ? F.webDarkTheme : F.webLightTheme}
          className={className}
        >
          {children}
        </F.FluentProvider>
      )}
    </Platform.Provider>
  );
}
// 保留 Fluent 的静态类型，页面无需知道 Material 自定义元素的属性细节。
function adapt<T extends React.ElementType>(
  desktop: T,
  mobile: (props: React.ComponentProps<T>) => React.ReactNode,
): T {
  return ((props: React.ComponentProps<T>) =>
    useContext(Platform)
      ? h(mobile as React.ComponentType<React.ComponentProps<T>>, props)
      : h(desktop, props)) as T;
}
export const Button = adapt(
  F.Button,
  ({ appearance, icon, children, size: _size, ...props }) =>
    h(
      appearance === "primary"
        ? "md-filled-button"
        : ["subtle", "transparent"].includes(appearance ?? "")
          ? "md-text-button"
          : "md-outlined-button",
      props as Record<string, unknown>,
      icon && <span slot="icon">{icon as React.ReactNode}</span>,
      children,
    ),
);
export const Field = adapt(F.Field, ({ label, hint, children }) => {
  const text = typeof label === "string" ? label : "";
  return (
    <FieldLabel.Provider value={text}>
      <div className="md-field">
        {children as React.ReactNode}
        {hint && (
          <div className="subtle md-hint">{hint as React.ReactNode}</div>
        )}
      </div>
    </FieldLabel.Provider>
  );
});
export const Input = adapt(F.Input, ({ onChange, value, readOnly, ...props }) =>
  h("md-outlined-text-field", {
    ...props,
    label: useContext(FieldLabel),
    value,
    readOnly,
    oninput: (e: Event) =>
      onChange?.(e as unknown as React.ChangeEvent<HTMLInputElement>, {
        value: (e.target as HTMLInputElement).value,
      }),
  }),
);
export const Textarea = adapt(
  F.Textarea,
  ({ onChange, resize: _resize, ...props }) =>
    h("md-outlined-text-field", {
      ...props,
      type: "textarea",
      label: useContext(FieldLabel),
      oninput: (e: Event) =>
        onChange?.(e as unknown as React.ChangeEvent<HTMLTextAreaElement>, {
          value: (e.target as HTMLTextAreaElement).value,
        }),
    }),
);
export const Select = adapt(F.Select, ({ onChange, children, ...props }) =>
  h(
    "md-outlined-select",
    {
      ...props,
      label: useContext(FieldLabel),
      "data-field": useContext(FieldLabel),
      onchange: (e: Event) =>
        onChange?.(e as unknown as React.ChangeEvent<HTMLSelectElement>, {
          value: (e.target as HTMLSelectElement).value,
        }),
    },
    React.Children.map(children, (child) => {
      if (
        !React.isValidElement<{ value: string; children: React.ReactNode }>(
          child,
        )
      )
        return null;
      return h(
        "md-select-option",
        { value: child.props.value, "data-value": child.props.value },
        <div slot="headline">{child.props.children}</div>,
      );
    }),
  ),
);
export const Checkbox = adapt(F.Checkbox, ({ onChange, label, ...props }) => (
  <label className="md-check">
    {h("md-checkbox", {
      ...props,
      onchange: (e: Event) =>
        onChange?.(e as unknown as React.ChangeEvent<HTMLInputElement>, {
          checked: (e.target as HTMLInputElement).checked,
        }),
    })}
    {label as React.ReactNode}
  </label>
));
export const Card = adapt(F.Card, ({ children, className }) => (
  <section className={`md-card ${className ?? ""}`}>
    {children as React.ReactNode}
  </section>
));
export const Title1 = adapt(F.Title1, ({ children }) => <h1>{children}</h1>);
export const Title2 = adapt(F.Title2, ({ children }) => <h2>{children}</h2>);
export const Badge = adapt(F.Badge, ({ children, color }) => (
  <span className={`md-badge md-badge--${color ?? "neutral"}`}>
    {children as React.ReactNode}
  </span>
));
export const MessageBar = adapt(F.MessageBar, ({ children, intent }) => (
  <div
    role={intent === "error" ? "alert" : "status"}
    className={`md-message ${intent}`}
  >
    {children as React.ReactNode}
  </div>
));
export const MessageBarBody = adapt(F.MessageBarBody, ({ children }) => (
  <span>{children as React.ReactNode}</span>
));
export const ProgressBar = adapt(F.ProgressBar, ({ value, ...props }) =>
  h("md-linear-progress", {
    ...props,
    value,
    indeterminate: value === undefined,
  }),
);
export const Spinner = adapt(F.Spinner, ({ label }) => (
  <div role="status">
    {h("md-circular-progress", { indeterminate: true })}
    {label as React.ReactNode}
  </div>
));
export const Accordion = adapt(F.Accordion, ({ children }) => (
  <div>{children}</div>
));
export const AccordionItem = adapt(F.AccordionItem, ({ children }) => {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <AccordionState.Provider
      value={{ open, id, toggle: () => setOpen((value) => !value) }}
    >
      <section className="md-details">{children}</section>
    </AccordionState.Provider>
  );
});
export const AccordionHeader = adapt(F.AccordionHeader, ({ children }) => {
  const { open, id, toggle } = useContext(AccordionState);
  return (
    <button
      className="md-details-header"
      aria-expanded={open}
      aria-controls={id}
      onClick={toggle}
    >
      {children as React.ReactNode}
      <span className="md-chevron" aria-hidden="true">
        ⌄
      </span>
    </button>
  );
});
export const AccordionPanel = adapt(
  F.AccordionPanel,
  ({ children, className }) => {
    const { open, id } = useContext(AccordionState);
    return (
      <div
        className="md-collapse"
        data-open={open}
        inert={!open}
        aria-hidden={!open}
      >
        <div className="md-collapse-inner">
          <div id={id} className={className}>
            {children as React.ReactNode}
          </div>
        </div>
      </div>
    );
  },
);
export const Tab = F.Tab;
export const TabList = adapt(
  F.TabList,
  ({ children, selectedValue, onTabSelect, vertical, disabled, ...props }) => {
    const tabs = React.Children.toArray(children).filter(
      React.isValidElement,
    ) as React.ReactElement<React.ComponentProps<typeof F.Tab>>[];
    const id = useId();
    if (vertical)
      return (
        <nav
          {...props}
          role="tablist"
          className="md-navigation"
          onKeyDown={(e) => {
            const buttons = [
              ...e.currentTarget.querySelectorAll<HTMLButtonElement>("button"),
            ];
            const index = buttons.indexOf(
              document.activeElement as HTMLButtonElement,
            );
            const offset = ["ArrowRight", "ArrowDown"].includes(e.key)
              ? 1
              : ["ArrowLeft", "ArrowUp"].includes(e.key)
                ? -1
                : 0;
            if (offset) {
              e.preventDefault();
              buttons[
                (index + offset + buttons.length) % buttons.length
              ]?.focus();
            }
          }}
        >
          {tabs.map((tab, index) => (
            <button
              key={String(tab.props.value)}
              id={`${id}-${index}`}
              role="tab"
              type="button"
              aria-selected={selectedValue === tab.props.value}
              disabled={disabled}
              onClick={(e) => onTabSelect?.(e, { value: tab.props.value })}
            >
              {h("md-ripple", { for: `${id}-${index}` })}
              {h("md-focus-ring", { for: `${id}-${index}` })}
              <span className="md-nav-icon">
                {tab.props.icon as React.ReactNode}
              </span>
              <span>{tab.props.children}</span>
            </button>
          ))}
        </nav>
      );
    return h(
      "md-tabs",
      {
        ...props,
        role: "tablist",
        inert: disabled,
        activeTabIndex: tabs.findIndex((t) => t.props.value === selectedValue),
        onchange: (e: Event) => {
          if (disabled) return;
          const i = (e.target as unknown as { activeTabIndex: number })
            .activeTabIndex;
          if (tabs[i])
            onTabSelect?.(e as unknown as React.MouseEvent<HTMLElement>, {
              value: tabs[i].props.value,
            });
        },
      },
      tabs.map((tab) =>
        h(
          "md-primary-tab",
          {
            key: String(tab.props.value),
            role: "tab",
            "aria-selected": selectedValue === tab.props.value,
            tabIndex: selectedValue === tab.props.value ? 0 : -1,
          },
          tab.props.children,
        ),
      ),
    );
  },
);
export const Dialog = adapt(F.Dialog, ({ open, onOpenChange, children }) =>
  h(
    "md-dialog",
    {
      open,
      quick: useReducedMotion(),
      "aria-hidden": !open,
      inert: !open,
      oncancel: (e: Event) => {
        e.preventDefault();
        onOpenChange?.(
          e as unknown as Parameters<
            NonNullable<React.ComponentProps<typeof F.Dialog>["onOpenChange"]>
          >[0],
          {
            open: false,
            type: "escapeKeyDown",
            event: e as unknown as React.KeyboardEvent<HTMLElement>,
          },
        );
      },
    },
    children,
  ),
);
export const DialogSurface = adapt(F.DialogSurface, ({ children }) => (
  <>{children as React.ReactNode}</>
));
export const DialogBody = adapt(F.DialogBody, ({ children }) => (
  <>{children as React.ReactNode}</>
));
export const DialogTitle = adapt(F.DialogTitle, ({ children }) => (
  <div slot="headline">{children as React.ReactNode}</div>
));
export const DialogContent = adapt(F.DialogContent, ({ children }) => (
  <div slot="content">{children as React.ReactNode}</div>
));
export const DialogActions = adapt(F.DialogActions, ({ children }) => (
  <div slot="actions">{children as React.ReactNode}</div>
));
