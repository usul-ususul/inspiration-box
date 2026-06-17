const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const shell = document.querySelector(".shell");
const panel = document.querySelector("#panel");
const quickInput = document.querySelector("#quickInput");
const content = document.querySelector("#content");
const statusEl = document.querySelector("#status");
const preview = document.querySelector("#preview");
const previewImage = document.querySelector("#previewImage");

let imageData = null;
let expanded = false;
let quickStep = "content";
let pendingQuickContent = "";

function setStatus(message, isError = false) {
  statusEl.textContent = message;
  statusEl.className = isError ? "status error" : "status";
}

function hexToRgba(hex, opacity) {
  const clean = String(hex || "#f8fafb").replace("#", "");
  const value = clean.length === 3
    ? clean.split("").map((char) => char + char).join("")
    : clean.padEnd(6, "f").slice(0, 6);
  const red = parseInt(value.slice(0, 2), 16);
  const green = parseInt(value.slice(2, 4), 16);
  const blue = parseInt(value.slice(4, 6), 16);
  const alpha = Math.min(1, Math.max(0.35, Number(opacity || 1)));
  return `rgba(${red}, ${green}, ${blue}, ${alpha})`;
}

function applyAppearance(settings) {
  shell.style.backgroundColor = hexToRgba(settings.windowColor, settings.windowOpacity);
  panel.style.backgroundColor = hexToRgba(settings.windowColor, settings.windowOpacity);
}

async function loadAppearance() {
  try {
    applyAppearance(await invoke("get_settings"));
  } catch (error) {
    console.warn(error);
  }
}

async function toggle(value = !expanded) {
  expanded = value;
  if (expanded) {
    panel.hidden = false;
    await invoke("set_expanded", { expanded: true });
    content.focus();
    return;
  }
  panel.hidden = true;
  await new Promise((resolve) => setTimeout(resolve, 30));
  await invoke("set_expanded", { expanded: false });
}

function resetQuickInput() {
  quickStep = "content";
  pendingQuickContent = "";
  quickInput.value = "";
  quickInput.placeholder = "Record...";
  quickInput.classList.remove("category-mode");
}

async function saveRecord(text, category = "") {
  try {
    setStatus("Saving...");
    await invoke("save_record", {
      input: { content: text, category, imageDataUrl: imageData },
    });
    resetQuickInput();
    content.value = "";
    imageData = null;
    preview.style.display = "none";
    setStatus("Saved. Syncing in background.");
    setTimeout(() => toggle(false), 500);
  } catch (error) {
    setStatus(String(error), true);
  }
}

async function saveCurrentRecord() {
  if (expanded) {
    await saveRecord(content.value, "");
    return;
  }
  if (quickStep === "content") {
    const text = quickInput.value.trim();
    if (!text && !imageData) {
      setStatus("Please enter content.");
      return;
    }
    pendingQuickContent = text;
    quickStep = "category";
    quickInput.value = "";
    quickInput.placeholder = "Category, Enter to skip";
    quickInput.classList.add("category-mode");
    setStatus("Enter a category, or press Enter to skip.");
    return;
  }
  await saveRecord(pendingQuickContent, quickInput.value.trim());
}

function loadImage(file) {
  if (!file || !file.type.startsWith("image/")) return;
  const reader = new FileReader();
  reader.onload = () => {
    imageData = reader.result;
    previewImage.src = imageData;
    preview.style.display = "block";
    toggle(true);
  };
  reader.readAsDataURL(file);
}

document.querySelector("#expand").onclick = () => {
  if (!expanded && quickStep === "content" && quickInput.value.trim()) {
    content.value = quickInput.value;
  }
  toggle();
};

document.querySelector("#sticky").onclick = async () => {
  try {
    await invoke("open_sticky_note");
    setStatus("Note opened.");
  } catch (error) {
    setStatus(`Note failed: ${error}`, true);
  }
};

document.querySelector("#details").onclick = async () => {
  await invoke("set_details_mode", { enabled: true });
  window.location.href = "details.html";
};

document.querySelector("#image").onclick = () => document.querySelector("#file").click();
document.querySelector("#file").onchange = (event) => loadImage(event.target.files[0]);
document.querySelector("#save").onclick = saveCurrentRecord;

document.querySelector("#dragHandle").addEventListener("mousedown", async (event) => {
  if (event.button !== 0) return;
  await appWindow.startDragging();
  setTimeout(() => invoke("snap_main_window"), 120);
});

quickInput.addEventListener("keydown", (event) => {
  if (event.key === "Escape" && quickStep === "category") {
    event.preventDefault();
    resetQuickInput();
    setStatus("Quick record canceled.");
    return;
  }
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    saveCurrentRecord();
  }
});

quickInput.addEventListener("paste", () => {
  setTimeout(() => {
    if (quickStep === "content" && !expanded && quickInput.value.length > 80) {
      content.value = quickInput.value;
      toggle(true);
    }
  }, 0);
});

document.addEventListener("paste", (event) => {
  const file = [...event.clipboardData.files][0];
  if (file) loadImage(file);
});
document.addEventListener("dragover", (event) => event.preventDefault());
document.addEventListener("drop", (event) => {
  event.preventDefault();
  loadImage(event.dataTransfer.files[0]);
});

listen("records-changed", async () => {
  if (expanded && !content.value.trim() && !imageData) {
    await toggle(false);
  }
});
listen("appearance-changed", (event) => applyAppearance(event.payload || {}));

resetQuickInput();
loadAppearance();
