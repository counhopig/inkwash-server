// navigator.clipboard is only exposed in secure contexts (https, or
// localhost) - this console is commonly opened over plain http:// on a
// LAN IP (e.g. http://192.168.1.10:8080), where it's undefined and a
// direct call throws. Fall back to the older execCommand('copy') path,
// which works regardless of secure-context status as long as it runs
// inside a real user-gesture handler (a click), which callers must ensure.
export async function copyText(text: string): Promise<boolean> {
  if (navigator.clipboard && window.isSecureContext) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // fall through to the legacy path
    }
  }
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.style.position = "fixed";
  ta.style.left = "-9999px";
  document.body.appendChild(ta);
  ta.focus();
  ta.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  document.body.removeChild(ta);
  return ok;
}
