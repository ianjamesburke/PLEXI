import { compileKeybinds } from "../shared/keybinds.js";

const platformName =
  globalThis.navigator?.userAgentData?.platform ||
  globalThis.navigator?.platform ||
  globalThis.navigator?.userAgent ||
  (process.platform === "darwin" ? "Mac" : process.platform);
const isMacOS = /\bMac/i.test(platformName);

const APP_KEYBIND_SPECS = isMacOS
  ? [
    "super+n=new_terminal_right",
    "ctrl+n=new_terminal_right",
    "super+shift+n=new_terminal_down",
    "ctrl+shift+n=new_terminal_down",
    "super+w=close_terminal",
    "ctrl+w=close_terminal",
    "super+s=save_workspace",
    "ctrl+s=save_workspace",
    "super+b=toggle_sidebar",
    "ctrl+b=toggle_sidebar",
    "super+m=toggle_minimap",
    "ctrl+m=toggle_minimap",
    "super+/=toggle_shortcuts",
    "ctrl+/=toggle_shortcuts",
    "super+equal=zoom_in",
    "super+minus=zoom_out",
    "ctrl+equal=zoom_in",
    "ctrl+minus=zoom_out",
    "super+left=focus_left",
    "ctrl+left=focus_left",
    "super+h=focus_left",
    "ctrl+h=focus_left",
    "super+right=focus_right",
    "ctrl+right=focus_right",
    "super+l=focus_right",
    "ctrl+l=focus_right",
    "super+up=focus_up",
    "ctrl+up=focus_up",
    "super+k=focus_up",
    "ctrl+k=focus_up",
    "super+down=focus_down",
    "ctrl+down=focus_down",
    "super+j=focus_down",
    "ctrl+j=focus_down",
    "super+1=context_1",
    "ctrl+1=context_1",
    "super+2=context_2",
    "ctrl+2=context_2",
    "super+3=context_3",
    "ctrl+3=context_3",
    "super+4=context_4",
    "ctrl+4=context_4",
    "super+5=context_5",
    "ctrl+5=context_5",
    "super+6=context_6",
    "ctrl+6=context_6",
    "super+7=context_7",
    "ctrl+7=context_7",
    "super+8=context_8",
    "ctrl+8=context_8",
    "super+9=context_9",
    "ctrl+9=context_9",
  ]
  : [
    "ctrl+n=new_terminal_right",
    "ctrl+shift+n=new_terminal_down",
    "ctrl+w=close_terminal",
    "ctrl+s=save_workspace",
    "ctrl+b=toggle_sidebar",
    "ctrl+m=toggle_minimap",
    "ctrl+/=toggle_shortcuts",
    "ctrl+equal=zoom_in",
    "ctrl+minus=zoom_out",
    "ctrl+left=focus_left",
    "ctrl+h=focus_left",
    "ctrl+right=focus_right",
    "ctrl+l=focus_right",
    "ctrl+up=focus_up",
    "ctrl+k=focus_up",
    "ctrl+down=focus_down",
    "ctrl+j=focus_down",
    "ctrl+1=context_1",
    "ctrl+2=context_2",
    "ctrl+3=context_3",
    "ctrl+4=context_4",
    "ctrl+5=context_5",
    "ctrl+6=context_6",
    "ctrl+7=context_7",
    "ctrl+8=context_8",
    "ctrl+9=context_9",
  ];

const TERMINAL_KEYBIND_SPECS = isMacOS
  ? [
    "performable:super+c=copy_to_clipboard",
    "super+v=paste_from_clipboard",
  ]
  : [
    "performable:ctrl+c=copy_to_clipboard",
    "ctrl+v=paste_from_clipboard",
  ];

export const APP_KEYBINDS = compileKeybinds(APP_KEYBIND_SPECS);
export const TERMINAL_KEYBINDS = compileKeybinds(TERMINAL_KEYBIND_SPECS);
