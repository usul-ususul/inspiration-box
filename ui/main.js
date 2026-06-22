const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;

const shell = document.querySelector(".shell");
const panel = document.querySelector("#panel");
const quickInput = document.querySelector("#quickInput");
const content = document.querySelector("#content");
const statusEl = document.querySelector("#status");
const preview = document.querySelector("#preview");
const previewImage = document.querySelector("#previewImage");
const dragHandle = document.querySelector("#dragHandle");
const moreMenu = document.querySelector("#moreMenu");

let imageData = null;
let expanded = false;
let moreOpen = false;
let quickStep = "content";
let pendingQuickContent = "";
let enterDirectSave = false;

/* 液态玻璃弹性鼠标跟随 */
let glassSpringX = 50, glassSpringY = 50;
let glassTargetX = 50, glassTargetY = 50;
let glassAnimId = null;
const GLASS_SPRING = 0.12;

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
  const alpha = Math.min(1, Math.max(0, Number(opacity || 1)));
  return `rgba(${red}, ${green}, ${blue}, ${alpha})`;
}

function applyAppearance(settings) {
  const rawAlpha = Number(settings.windowOpacity || 1);
  const glass = Boolean(settings.glassMode);

  if (glass) {
    // 玻璃模式：完全覆盖默认样式,不设 background-color
    shell.style.backgroundColor = "";
    panel.style.backgroundColor = "";
    document.documentElement.style.setProperty("--shell-border", "rgba(255,255,255,0.25)");
  } else {
    shell.style.backgroundColor = hexToRgba(settings.windowColor, rawAlpha);
    panel.style.backgroundColor = hexToRgba(settings.windowColor, rawAlpha);
    document.documentElement.style.setProperty("--shell-border",
      rawAlpha < 0.05 ? "transparent" : hexToRgba(settings.windowColor, rawAlpha));
  }

  document.documentElement.dataset.glass = glass ? "true" : "false";
  document.documentElement.dataset.shadowless = "true";
  document.documentElement.dataset.moreTransparent = settings.moreTransparent ? "true" : "false";
  document.documentElement.dataset.inputTransparent = settings.inputTransparent ? "true" : "false";
  document.documentElement.dataset.textStroke = settings.textStroke ? "true" : "false";
  enterDirectSave = Boolean(settings.enterDirectSave);
  // 玻璃模式开关时启停弹性跟随
  if (glass && !glassAnimId) startGlassGlow();
  else if (!glass) stopGlassGlow();
}

/* ====== 液态玻璃弹性鼠标跟随 ====== */

function updateGlassGlow() {
  const el = document.getElementById("glassGlow");
  if (!el || document.documentElement.dataset.glass !== "true") {
    glassAnimId = null;
    return;
  }
  // 弹簧插值
  glassSpringX += (glassTargetX - glassSpringX) * GLASS_SPRING;
  glassSpringY += (glassTargetY - glassSpringY) * GLASS_SPRING;
  el.style.setProperty("--glow-x", glassSpringX + "%");
  el.style.setProperty("--glow-y", glassSpringY + "%");
  glassAnimId = requestAnimationFrame(updateGlassGlow);
}

function startGlassGlow() {
  if (glassAnimId) return;
  glassSpringX = glassTargetX;
  glassSpringY = glassTargetY;
  glassAnimId = requestAnimationFrame(updateGlassGlow);
}

function stopGlassGlow() {
  if (glassAnimId) {
    cancelAnimationFrame(glassAnimId);
    glassAnimId = null;
  }
}

// 仅当 glass 模式激活时追踪鼠标
document.addEventListener("mousemove", (event) => {
  if (document.documentElement.dataset.glass !== "true") {
    stopGlassGlow();
    return;
  }
  const shell = document.querySelector(".shell");
  if (!shell) return;
  const rect = shell.getBoundingClientRect();
  glassTargetX = ((event.clientX - rect.left) / rect.width) * 100;
  glassTargetY = ((event.clientY - rect.top) / rect.height) * 100;
  startGlassGlow();
}, { passive: true });

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
    await invoke("set_expanded", { expanded: true, moreOpen });
    content.focus();
    return;
  }
  panel.hidden = true;
  await new Promise((resolve) => setTimeout(resolve, 30));
  await invoke("set_expanded", { expanded: false, moreOpen });
}

function syncExpandMenuLabel() {
  const label = moreMenu.querySelector("[data-action='expand'] .more-item-label");
  if (!label) return;
  label.textContent = expanded ? "收起长文本" : "长文本";
}

function setMoreOpen(value) {
  moreOpen = value;
  if (value) {
    const rect = dragHandle.getBoundingClientRect();
    moreMenu.style.left = `${rect.left}px`;
    moreMenu.style.top = `${rect.bottom + 2}px`;
    moreMenu.hidden = false;
    dragHandle.classList.add("active");
    dragHandle.setAttribute("aria-expanded", "true");
    const firstItem = moreMenu.querySelector(".more-item");
    if (firstItem) firstItem.focus();
    // 菜单弹出时需要增大窗口高度,避免被窗口边界裁剪
    invoke("set_expanded", { expanded, moreOpen: true }).catch(() => {});
  } else {
    moreMenu.hidden = true;
    dragHandle.classList.remove("active");
    dragHandle.setAttribute("aria-expanded", "false");
    invoke("set_expanded", { expanded, moreOpen: false }).catch(() => {});
  }
}

function toggleMore(force) {
  setMoreOpen(typeof force === "boolean" ? force : !moreOpen);
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
    if (enterDirectSave) {
      // 开启"回车直接保存" → 不进入类别选择,直接保存
      await saveRecord(text, "");
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

document.querySelector("#save").onclick = saveCurrentRecord;
document.querySelector("#image").onclick = () => document.querySelector("#file").click();
document.querySelector("#file").onchange = (event) => loadImage(event.target.files[0]);

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

/* ====== 拖动 / 点击（拖动手柄即"更多"开关） ======
   不再使用 data-tauri-drag-region（Tauri v2 会拦截指针事件）。
   改用 mousedown / mousemove / mouseup 手动判断：
   - 位移 < 5px 视为点击 → 切换"更多"菜单
   - 位移 >= 5px 视为拖动 → 调用 Rust drag_window 启动原生拖拽  */
const DRAG_CLICK_THRESHOLD = 5;
let dragMouseDown = false;
let dragStartX = 0, dragStartY = 0;
let dragMoved = false;
let dragStarted = false;  // 是否已调用 drag_window

dragHandle.addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  dragMouseDown = true;
  dragStartX = event.clientX;
  dragStartY = event.clientY;
  dragMoved = false;
  dragStarted = false;
});

document.addEventListener("mousemove", (event) => {
  if (!dragMouseDown) return;
  const dx = event.clientX - dragStartX;
  const dy = event.clientY - dragStartY;
  if (Math.abs(dx) > DRAG_CLICK_THRESHOLD || Math.abs(dy) > DRAG_CLICK_THRESHOLD) {
    dragMoved = true;
    if (!dragStarted) {
      dragStarted = true;
      invoke("drag_window").catch(() => {});
    }
  }
});

document.addEventListener("mouseup", (event) => {
  if (event.button !== 0) return;
  if (!dragMouseDown) return;
  dragMouseDown = false;
  if (!dragMoved) {
    // 纯点击 → 关闭其他弹出菜单后切换"更多"
    if (moreOpen) {
      setMoreOpen(false);
    } else {
      toggleMore();
    }
    // 不 setMoreOpen(false) 后再 toggleMore() —— 已在 toggleMore 里处理
  }
});

dragHandle.addEventListener("keydown", (event) => {
  if (event.key === "Enter" || event.key === " ") {
    event.preventDefault();
    toggleMore();
  } else if (event.key === "Escape" && moreOpen) {
    setMoreOpen(false);
    dragHandle.focus();
  }
});

/* ====== "更多"菜单项动作 ====== */
async function runMoreAction(action) {
  setMoreOpen(false);
  switch (action) {
    case "details": {
      try {
        await invoke("set_details_mode", { enabled: true });
        window.location.href = "details.html";
      } catch (error) {
        setStatus(String(error), true);
      }
      return;
    }
    case "sticky":
      try {
        await invoke("open_sticky_note");
        setStatus("Note opened.");
      } catch (error) {
        setStatus(`Note failed: ${error}`, true);
      }
      return;
    case "expand":
      if (!expanded && quickStep === "content" && quickInput.value.trim()) {
        content.value = quickInput.value;
      }
      await toggle();
      syncExpandMenuLabel();
      return;
    case "quit":
      try {
        await invoke("quit_app");
      } catch (error) {
        setStatus(String(error), true);
      }
      return;
  }
}

moreMenu.addEventListener("click", (event) => {
  const item = event.target.closest(".more-item");
  if (!item) return;
  runMoreAction(item.dataset.action);
});

moreMenu.addEventListener("keydown", (event) => {
  const items = [...moreMenu.querySelectorAll(".more-item")];
  const idx = items.indexOf(document.activeElement);
  if (event.key === "ArrowDown") {
    event.preventDefault();
    items[(idx + 1 + items.length) % items.length].focus();
  } else if (event.key === "ArrowUp") {
    event.preventDefault();
    items[(idx - 1 + items.length) % items.length].focus();
  } else if (event.key === "Escape") {
    event.preventDefault();
    setMoreOpen(false);
    dragHandle.focus();
  }
});

/* 点击菜单外区域关闭 */
document.addEventListener("mousedown", (event) => {
  if (!moreOpen) return;
  if (event.target === dragHandle || dragHandle.contains(event.target)) return;
  if (moreMenu.contains(event.target)) return;
  setMoreOpen(false);
}, true);

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
    syncExpandMenuLabel();
  }
});
listen("appearance-changed", (event) => applyAppearance(event.payload || {}));
listen("summon-floating-bar", () => {
  quickInput.focus();
  quickInput.select();
});

resetQuickInput();
loadAppearance();
moreMenu.hidden = true;
syncExpandMenuLabel();
invoke("set_expanded", { expanded, moreOpen });
if (sessionStorage.getItem("focusQuickInput") === "1") {
  sessionStorage.removeItem("focusQuickInput");
  requestAnimationFrame(() => quickInput.focus());
}
