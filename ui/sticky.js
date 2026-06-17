const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const appWindow = window.__TAURI__.window.getCurrentWindow();

const stickyNote = document.querySelector("#stickyNote");
const stickyStatus = document.querySelector("#stickyStatus");
const stickyMenu = document.querySelector("#stickyMenu");
const stickyMenuButton = document.querySelector("#stickyMenuButton");
let saveTimer = null;
let pinned = true;
let stickyMode = "free";
let stickyEdge = "nearest";
let collapseTimer = null;
let isDraggingSticky = false;
const colorKey = "inspiration-box.sticky-color";
const defaultColor = "purple";

function setStickyStatus(message, isError = false) {
  stickyStatus.textContent = message;
  stickyStatus.className = isError ? "sticky-status error" : "sticky-status";
}

function noteText() {
  return stickyNote.innerText.replace(/ /g, " ").trim();
}

function updateFormatStates() {
  document.querySelectorAll("[data-command]").forEach((btn) => {
    try {
      btn.classList.toggle("active", document.queryCommandState(btn.dataset.command));
    } catch (_) {}
  });
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
  template.content.querySelectorAll("img").forEach((image) => {
    const source = image.getAttribute("src") || "";
    if (!source.startsWith("data:image/")) {
      image.remove();
      return;
    }
    image.removeAttribute("width");
    image.removeAttribute("height");
    image.removeAttribute("style");
    image.alt = image.alt || "sticky image";
  });
  return template.innerHTML;
}

function applyStickyColor(colorName) {
  const color = colorName || defaultColor;
  document.body.dataset.stickyColor = color;
  localStorage.setItem(colorKey, color);
  document.querySelectorAll(".sticky-swatch").forEach((button) => {
    button.classList.toggle("active", button.dataset.color === color);
  });
}

function setModeButtons(mode) {
  document.querySelectorAll(".sticky-mode-option").forEach((button) => {
    button.classList.toggle("active", button.dataset.mode === mode);
  });
}

function clearCollapseTimer() {
  clearTimeout(collapseTimer);
  collapseTimer = null;
}

async function expandFromEdge() {
  if (stickyMode !== "edge") return;
  clearCollapseTimer();
  stickyEdge = await invoke("set_sticky_edge_state", {
    edge: stickyEdge,
    collapsed: false,
  });
  document.body.classList.remove("sticky-collapsed");
}

function scheduleCollapse() {
  if (stickyMode !== "edge" || isDraggingSticky) return;
  clearCollapseTimer();
  collapseTimer = setTimeout(async () => {
    try {
      stickyEdge = await invoke("set_sticky_edge_state", {
        edge: stickyEdge,
        collapsed: true,
      });
      document.body.classList.add("sticky-collapsed");
    } catch (error) {
      setStickyStatus(String(error), true);
    }
  }, 500);
}

async function applyStickyMode(mode, persist = false) {
  stickyMode = mode === "edge" ? "edge" : "free";
  setModeButtons(stickyMode);
  document.body.dataset.stickyMode = stickyMode;
  clearCollapseTimer();

  if (persist) {
    await invoke("set_sticky_mode", { mode: stickyMode });
  }

  if (stickyMode === "edge") {
    stickyEdge = await invoke("set_sticky_edge_state", {
      edge: "nearest",
      collapsed: true,
    });
    document.body.classList.add("sticky-collapsed");
    // 额外延迟确保窗口位置在显示器切换等场景下正确
    setTimeout(() => {
      if (stickyMode === "edge") {
        invoke("set_sticky_edge_state", {
          edge: stickyEdge,
          collapsed: true,
        })
          .then((edge) => {
            stickyEdge = edge;
            document.body.classList.add("sticky-collapsed");
          })
          .catch((error) => setStickyStatus(String(error), true));
      }
    }, 700);
    setTimeout(() => {
      if (stickyMode === "edge") {
        invoke("set_sticky_edge_state", {
          edge: stickyEdge,
          collapsed: true,
        })
          .then((edge) => {
            stickyEdge = edge;
            document.body.classList.add("sticky-collapsed");
          })
          .catch((error) => setStickyStatus(String(error), true));
      }
    }, 1500);
    setStickyStatus("已切换到吸边隐藏");
    return;
  }

  await invoke("set_sticky_edge_state", {
    edge: stickyEdge,
    collapsed: false,
  });
  document.body.classList.remove("sticky-collapsed");
  setStickyStatus("已切换到自由拖动");
}

async function saveStickyNow() {
  await invoke("save_sticky_note", {
    content: sanitizeStickyHtml(stickyNote.innerHTML),
  });
}

async function loadStickyNote() {
  try {
    stickyNote.innerHTML = sanitizeStickyHtml(await invoke("get_sticky_note"));
    setStickyStatus("已保存到本地");
    stickyNote.focus();
  } catch (error) {
    setStickyStatus(String(error), true);
  }
}

function queueSave() {
  clearTimeout(saveTimer);
  setStickyStatus("正在保存...");
  saveTimer = setTimeout(async () => {
    try {
      await saveStickyNow();
      setStickyStatus("已保存");
    } catch (error) {
      setStickyStatus(String(error), true);
    }
  }, 350);
}

stickyNote.addEventListener("input", queueSave);
stickyNote.addEventListener("paste", (event) => {
  const imageItem = [...event.clipboardData.items].find((item) => item.type.startsWith("image/"));
  if (imageItem) {
    event.preventDefault();
    const file = imageItem.getAsFile();
    const reader = new FileReader();
    reader.onload = () => {
      stickyNote.focus();
      document.execCommand("insertHTML", false, `<img src="${reader.result}" alt="sticky image">`);
      queueSave();
    };
    reader.readAsDataURL(file);
    return;
  }

  event.preventDefault();
  document.execCommand("insertText", false, event.clipboardData.getData("text/plain"));
});

// ====== 拖拽：纯 JS 实现，手动检测避开按钮 ======
document.querySelector("#stickyDrag").addEventListener("mousedown", (event) => {
  if (event.button !== 0) return;
  // 手动遍历 DOM 树检测是否点击在按钮上（不用 closest，避免 WebView2 兼容问题）
  let el = event.target;
  while (el && el !== event.currentTarget) {
    if (el.tagName === "BUTTON") return;
    el = el.parentElement;
  }
  isDraggingSticky = true;
  expandFromEdge().then(() => {
    appWindow.startDragging().finally(() => {
      isDraggingSticky = false;
      if (stickyMode === "edge") {
        stickyEdge = "nearest";
        scheduleCollapse();
      }
    });
  });
});

// ====== 标题栏按钮事件 ======
// 暴露给 HTML 内联 onclick 使用
window.hideSticky = () => {
  appWindow.hide();
};
window.minimizeSticky = () => {
  appWindow.minimize();
};

stickyMenuButton.onclick = (event) => {
  event.stopPropagation();
  stickyMenu.hidden = !stickyMenu.hidden;
};

document.addEventListener("click", (event) => {
  if (!event.target.closest(".sticky-menu") && event.target !== stickyMenuButton) {
    stickyMenu.hidden = true;
  }
});

document.querySelector("#pinSticky").onclick = async (event) => {
  pinned = !pinned;
  await invoke("set_sticky_pinned", { pinned });
  event.currentTarget.classList.toggle("active", pinned);
  setStickyStatus(pinned ? "已置顶" : "已取消置顶");
};

// ====== 格式化工具栏按钮 ======
document.querySelectorAll("[data-command]").forEach((button) => {
  button.onclick = () => {
    stickyNote.focus();
    document.execCommand(button.dataset.command, false, null);
    updateFormatStates();
    queueSave();
  };
});

// ====== 颜色按钮 ======
document.querySelectorAll(".sticky-swatch").forEach((button) => {
  button.onclick = () => {
    applyStickyColor(button.dataset.color);
    stickyMenu.hidden = true;
    setStickyStatus("已更换颜色");
    stickyNote.focus();
  };
});

// ====== 模式切换按钮 ======
document.querySelectorAll(".sticky-mode-option").forEach((button) => {
  button.onclick = async () => {
    try {
      await applyStickyMode(button.dataset.mode, true);
      stickyMenu.hidden = true;
      stickyNote.focus();
    } catch (error) {
      setStickyStatus(String(error), true);
    }
  };
});

// ====== 截图按钮 ======
document.querySelector("#screenShot").onclick = () => {
  invoke("open_screen_clip")
    .then(() => setStickyStatus("截图后按 Ctrl+V 粘贴到便签"))
    .catch((error) => setStickyStatus(String(error), true));
};

// ====== 吸边悬浮展开/折叠 ======
document.body.addEventListener("mouseenter", () => {
  expandFromEdge().catch((error) => setStickyStatus(String(error), true));
});

document.body.addEventListener("mouseleave", () => {
  scheduleCollapse();
});

// ====== 监听模式变化事件 ======
listen("sticky-mode-changed", (event) => {
  applyStickyMode(event.payload?.mode || "free", false).catch((error) => {
    setStickyStatus(String(error), true);
  });
});

// ====== 格式按钮状态更新 ======
document.addEventListener("selectionchange", () => {
  const sel = document.getSelection();
  if (sel && sel.rangeCount > 0 && stickyNote.contains(sel.anchorNode)) {
    updateFormatStates();
  }
});

// ====== 快捷键 ======
document.addEventListener("keydown", (e) => {
  if (!e.ctrlKey && !e.metaKey) return;
  const key = e.key.toLowerCase();
  if (key === "b" && !e.shiftKey) {
    e.preventDefault();
    document.execCommand("bold");
  } else if (key === "i" && !e.shiftKey) {
    e.preventDefault();
    document.execCommand("italic");
  } else if (key === "u" && !e.shiftKey) {
    e.preventDefault();
    document.execCommand("underline");
  } else if (key === "x" && e.shiftKey) {
    e.preventDefault();
    document.execCommand("strikeThrough");
  } else if (key === "s" && !e.shiftKey) {
    e.preventDefault();
    document.querySelector("#toRecord").click();
    return;
  } else {
    return;
  }
  updateFormatStates();
  queueSave();
});

// ====== 转为灵感记录 ======
document.querySelector("#toRecord").onclick = async () => {
  clearTimeout(saveTimer);
  try {
    if (!noteText()) {
      throw new Error("便签为空，不能转为灵感");
    }
    await saveStickyNow();
    await invoke("sticky_to_record");
    setStickyStatus("已转为灵感，正在同步 Notion");
  } catch (error) {
    setStickyStatus(String(error), true);
  }
};

// ====== 初始化：颜色 + 模式 + 加载内容 ======
applyStickyColor(localStorage.getItem(colorKey) || defaultColor);
const initialMode = new URLSearchParams(window.location.search).get("mode");
(initialMode
  ? Promise.resolve(initialMode)
  : invoke("get_sticky_mode"))
  .then((mode) => applyStickyMode(mode, false))
  .catch((error) => setStickyStatus(String(error), true));
loadStickyNote();
