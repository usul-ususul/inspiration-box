const invoke = window.__TAURI__.core.invoke;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const stickyNote = document.querySelector("#stickyNote");
const stickyStatus = document.querySelector("#stickyStatus");
let saveTimer = null;
let pinned = true;

function setStickyStatus(message, isError = false) {
  stickyStatus.textContent = message;
  stickyStatus.className = isError ? "sticky-status error" : "sticky-status";
}

function noteText() {
  return stickyNote.innerText.replace(/\u00a0/g, " ").trim();
}

function sanitizeStickyHtml(html) {
  const template = document.createElement("template");
  template.innerHTML = html || "";
  template.content.querySelectorAll("script,style,iframe,object,embed,link,meta").forEach((node) => {
    node.remove();
  });
  template.content.querySelectorAll("*").forEach((node) => {
    [...node.attributes].forEach((attribute) => {
      const name = attribute.name.toLowerCase();
      const value = attribute.value.toLowerCase();
      if (name.startsWith("on") || value.includes("javascript:")) {
        node.removeAttribute(attribute.name);
      }
    });
  });
  return template.innerHTML;
}

async function saveStickyNow() {
  await invoke("save_sticky_note", {
    content: sanitizeStickyHtml(stickyNote.innerHTML),
  });
}

async function loadStickyNote() {
  try {
    stickyNote.innerHTML = sanitizeStickyHtml(await invoke("get_sticky_note"));
    setStickyStatus("\u5df2\u4fdd\u5b58\u5230\u672c\u5730");
    stickyNote.focus();
  } catch (error) {
    setStickyStatus(String(error), true);
  }
}

function queueSave() {
  clearTimeout(saveTimer);
  setStickyStatus("\u6b63\u5728\u4fdd\u5b58...");
  saveTimer = setTimeout(async () => {
    try {
      await saveStickyNow();
      setStickyStatus("\u5df2\u4fdd\u5b58");
    } catch (error) {
      setStickyStatus(String(error), true);
    }
  }, 350);
}

stickyNote.addEventListener("input", queueSave);
stickyNote.addEventListener("paste", (event) => {
  event.preventDefault();
  document.execCommand("insertText", false, event.clipboardData.getData("text/plain"));
});

document.querySelector("#stickyDrag").addEventListener("mousedown", async (event) => {
  if (event.button !== 0 || event.target.closest("button")) return;
  await appWindow.startDragging();
});

document.querySelector("#closeSticky").onclick = () => appWindow.hide();
document.querySelector("#minSticky").onclick = () => appWindow.minimize();

document.querySelector("#pinSticky").onclick = async (event) => {
  pinned = !pinned;
  await invoke("set_sticky_pinned", { pinned });
  event.currentTarget.classList.toggle("active", pinned);
  setStickyStatus(pinned ? "\u5df2\u7f6e\u9876" : "\u5df2\u53d6\u6d88\u7f6e\u9876");
};

document.querySelectorAll("[data-command]").forEach((button) => {
  button.onclick = () => {
    stickyNote.focus();
    document.execCommand(button.dataset.command, false, null);
    queueSave();
  };
});

document.querySelector("#screenShot").onclick = () => {
  stickyNote.focus();
  document.execCommand("insertText", false, "[\u5c4f\u5e55\u622a\u56fe\u5f85\u6dfb\u52a0]");
  queueSave();
};

document.querySelector("#toRecord").onclick = async () => {
  clearTimeout(saveTimer);
  try {
    if (!noteText()) {
      throw new Error("\u4fbf\u7b7e\u4e3a\u7a7a\uff0c\u4e0d\u80fd\u8f6c\u4e3a\u7075\u611f");
    }
    await saveStickyNow();
    await invoke("sticky_to_record");
    setStickyStatus("\u5df2\u8f6c\u4e3a\u7075\u611f\uff0c\u6b63\u5728\u540c\u6b65 Notion");
  } catch (error) {
    setStickyStatus(String(error), true);
  }
};

loadStickyNote();
