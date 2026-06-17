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
    setStickyStatus("Saved locally");
    stickyNote.focus();
  } catch (error) {
    setStickyStatus(String(error), true);
  }
}

function queueSave() {
  clearTimeout(saveTimer);
  setStickyStatus("Saving...");
  saveTimer = setTimeout(async () => {
    try {
      await saveStickyNow();
      setStickyStatus("Saved");
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
  setStickyStatus(pinned ? "Pinned" : "Unpinned");
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
  document.execCommand("insertText", false, "[Screenshot placeholder]");
  queueSave();
};

document.querySelector("#toRecord").onclick = async () => {
  clearTimeout(saveTimer);
  try {
    if (!noteText()) {
      throw new Error("Sticky note is empty");
    }
    await saveStickyNow();
    await invoke("sticky_to_record");
    setStickyStatus("Converted. Syncing to Notion");
  } catch (error) {
    setStickyStatus(String(error), true);
  }
};

loadStickyNote();
