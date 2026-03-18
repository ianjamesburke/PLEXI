/**
 * keys.js — centralized keyboard shortcut rendering.
 * Use `shortcut(...)` to produce an HTML string of styled <kbd> chips.
 * Keys are passed as individual strings; modifier aliases are normalised.
 *
 * Example:
 *   shortcut("⌘", "Shift", "N")  →  <span class="shortcut">…</span>
 */

const KEY_LABELS = {
  cmd: "⌘",
  command: "⌘",
  super: "⌘",
  "⌘": "⌘",
  ctrl: "⌃",
  control: "⌃",
  "⌃": "⌃",
  alt: "⌥",
  option: "⌥",
  "⌥": "⌥",
  shift: "⇧",
  "⇧": "⇧",
  "⇧": "⇧",
};

/**
 * Render a keyboard shortcut as an HTML string.
 * @param {...string} keys - Individual key names, e.g. "⌘", "Shift", "N"
 * @returns {string} HTML string
 */
export function shortcut(...keys) {
  const chips = keys
    .map((k) => {
      const label = KEY_LABELS[k.toLowerCase()] ?? KEY_LABELS[k] ?? k.toUpperCase();
      const isMod = label.length === 1 && "⌘⌃⌥⇧".includes(label);
      const cls = isMod ? "key key--mod" : "key";
      return `<kbd class="${cls}">${label}</kbd>`;
    })
    .join("");
  return `<span class="shortcut">${chips}</span>`;
}
