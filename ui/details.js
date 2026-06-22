const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const convertFileSrc = window.__TAURI__.core.convertFileSrc;
const appWindow = window.__TAURI__.window.getCurrentWindow();
const recordsEl = document.querySelector("#records");
const noticeEl = document.querySelector("#detailNotice");
const sectionTabs = document.querySelectorAll(".section-tab");
const sections = {
  app: document.querySelector("#appSection"),
  records: document.querySelector("#recordsSection"),
  sticky: document.querySelector("#stickySection"),
};
const stickyEditor = document.querySelector("#stickyEditor");
const stickySettingsStatus = document.querySelector("#stickySettingsStatus");
const stickyModeButtons = document.querySelectorAll("#stickyModeControl .segment");
const checkUpdateButton = document.querySelector("#checkUpdate");
const updateStatus = document.querySelector("#updateStatus");
const summonShortcutInput = document.querySelector("#summonShortcut");
const shortcutStatus = document.querySelector("#shortcutStatus");

const statusName = {
  pending: "Pending",
  failed: "Failed",
  synced: "Synced",
};

let stickySaveTimer = null;
let stickyLoaded = false;
let stickyModeLoaded = false;
let availableUpdateVersion = null;

function showAvailableUpdate(version) {
  availableUpdateVersion = version;
  updateStatus.textContent = `发现新版本 ${version}。`;
  checkUpdateButton.textContent = "立即更新";
}

async function installAvailableUpdate() {
  if (!window.confirm(`安装 ahhhh mmt ${availableUpdateVersion} 并重启应用？`)) return;
  checkUpdateButton.disabled = true;
  checkUpdateButton.textContent = "正在更新...";
  updateStatus.textContent = "正在下载并验证更新，请不要关闭应用。";
  try {
    await invoke("install_update");
  } catch (error) {
    updateStatus.textContent = `更新失败：${String(error)}`;
    checkUpdateButton.disabled = false;
    checkUpdateButton.textContent = "重新手动检测";
  }
}

checkUpdateButton.addEventListener("click", async () => {
  if (availableUpdateVersion) {
    await installAvailableUpdate();
    return;
  }

  checkUpdateButton.disabled = true;
  checkUpdateButton.textContent = "检查中...";
  updateStatus.textContent = "正在连接 GitHub Release...";
  try {
    const result = await invoke("check_for_update");
    if (result.available) {
      showAvailableUpdate(result.version);
    } else {
      updateStatus.textContent = `当前已是最新版本 ${result.currentVersion}。`;
      checkUpdateButton.textContent = "再次手动检测";
    }
  } catch (error) {
    updateStatus.textContent = `检查失败：${String(error)}`;
    checkUpdateButton.textContent = "重新手动检测";
  } finally {
    checkUpdateButton.disabled = false;
  }
});

function showNotice(message) {
  if (noticeEl) noticeEl.textContent = message;
}

function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text || "";
  return div.innerHTML;
}

function highlightSearchText(text, query) {
  const source = String(text || "");
  const needle = String(query || "").trim();
  if (!needle) return escapeHtml(source);

  const sourceLower = source.toLocaleLowerCase();
  const needleLower = needle.toLocaleLowerCase();
  const parts = [];
  let cursor = 0;
  let matchIndex = sourceLower.indexOf(needleLower, cursor);

  while (matchIndex !== -1) {
    parts.push(escapeHtml(source.slice(cursor, matchIndex)));
    parts.push(`<mark class="search-highlight">${escapeHtml(
      source.slice(matchIndex, matchIndex + needle.length),
    )}</mark>`);
    cursor = matchIndex + needle.length;
    matchIndex = sourceLower.indexOf(needleLower, cursor);
  }
  parts.push(escapeHtml(source.slice(cursor)));
  return parts.join("");
}

async function runAction(message, action) {
  showNotice(message);
  try {
    await action();
  } catch (error) {
    showNotice(`Error: ${String(error)}`);
  }
}

async function goBack() {
  const actionsExpanded = sessionStorage.getItem("quickActionsExpanded") === "1";
  await invoke("set_details_mode", { enabled: false, actionsExpanded });
  window.location.href = "index.html";
}

appWindow.onFocusChanged(({ payload: focused }) => {
  if (focused) return;
  setTimeout(async () => {
    if (!(await appWindow.isFocused())) {
      await goBack();
    }
  }, 150);
});

function showSection(name) {
  sectionTabs.forEach((tab) => {
    tab.classList.toggle("active", tab.dataset.section === name);
  });
  Object.entries(sections).forEach(([sectionName, section]) => {
    section.hidden = sectionName !== name;
  });
  if (name === "records") load();
  if (name === "sticky") loadSticky();
}

function setStickyModeButtons(mode) {
  stickyModeButtons.forEach((button) => {
    button.classList.toggle("active", button.dataset.mode === mode);
  });
}

async function load() {
  try {
    const searchQuery = document.querySelector("#search").value.trim();
    const records = await invoke("list_records", {
      search: searchQuery,
    });

    if (!records.length) {
      recordsEl.innerHTML = "<p>No records yet.</p>";
      return;
    }

    recordsEl.innerHTML = records.map((record) => {
      const actionText = record.action === "delete"
        ? "Deleted"
        : record.action === "correction"
          ? "Correction"
          : "";
      const imageHtml = record.image_path
        ? `<img src="${convertFileSrc(record.image_path)}" alt="record image">`
        : "";
      const retryHtml = record.status !== "synced"
        ? `<button class="secondary" type="button" data-action="retry">重试同步</button>`
        : "";

      return `<article class="record" data-id="${record.id}">
        <div class="meta">
          <span>${new Date(record.created_at).toLocaleString()}</span>
          <span>${statusName[record.status] || record.status}</span>
          <span>${actionText}</span>
        </div>
        ${record.category ? `<span class="category">${highlightSearchText(record.category, searchQuery)}</span>` : ""}
        <div class="content">${highlightSearchText(record.content, searchQuery)}</div>
        ${imageHtml}
        ${record.error ? `<p class="error">${escapeHtml(record.error)}</p>` : ""}
        <div class="record-actions">
          <button class="secondary" type="button" data-action="edit">编辑</button>
          <button class="secondary" type="button" data-action="to-sticky">转便签</button>
          <button class="secondary" type="button" data-action="delete">删除</button>
          ${retryHtml}
        </div>
      </article>`;
    }).join("");
  } catch (error) {
    recordsEl.innerHTML = `<p class="error">${escapeHtml(String(error))}</p>`;
  }
}

recordsEl.addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  if (!button) return;

  const article = button.closest(".record");
  if (!article) {
    showNotice("Error: record card not found");
    return;
  }

  const id = article.dataset.id;
  const action = button.dataset.action;

  if (action === "edit") {
    const contentEl = article.querySelector(".content");
    const oldText = contentEl.textContent;
    contentEl.innerHTML = `<textarea class="inline-editor">${escapeHtml(oldText)}</textarea>`;
    article.querySelector(".record-actions").innerHTML = `
      <button class="secondary" type="button" data-action="save-edit">保存</button>
      <button class="secondary" type="button" data-action="cancel-edit">取消</button>
    `;
    const editor = article.querySelector(".inline-editor");
    editor.focus();
    editor.selectionStart = editor.value.length;
    showNotice("Editing...");
    return;
  }

  if (action === "save-edit") {
    await runAction("Saving edit...", async () => {
      const editor = article.querySelector(".inline-editor");
      await invoke("update_record", { id, content: editor.value });
      showNotice("Edited.");
      await load();
    });
    return;
  }

  if (action === "cancel-edit") {
    await load();
    showNotice("Edit canceled.");
    return;
  }

  if (action === "delete") {
    await runAction("Deleting...", async () => {
      await invoke("delete_record", { id });
      showNotice("Delete queued.");
      await load();
    });
    return;
  }

  if (action === "to-sticky") {
    await runAction("Converting to sticky note...", async () => {
      await invoke("record_to_sticky", { id });
      await invoke("open_sticky_note");
      showNotice("已转为便签。");
    });
    return;
  }

  if (action === "retry") {
    await runAction("Queueing retry...", async () => {
      await invoke("retry_record", { id });
      showNotice("Retry queued.");
      await load();
    });
  }
});

async function loadSettings() {
  const data = await invoke("get_settings");
  document.querySelector("#pageId").value = data.pageId || "";
  document.querySelector("#autostart").checked = Boolean(data.autostart);
  document.querySelector("#windowColor").value = data.windowColor || "#f8fafb";
  document.querySelector("#windowOpacity").value = data.windowOpacity || "1";
  document.querySelector("#opacityValue").textContent =
    `${Math.round(Number(document.querySelector("#windowOpacity").value) * 100)}%`;
  document.querySelector("#shadowless").checked = true; // 阴影始终关闭（新行为）
  document.querySelector("#moreTransparent").checked = Boolean(data.moreTransparent);
  document.querySelector("#inputTransparent").checked = Boolean(data.inputTransparent);
  document.querySelector("#enterDirectSave").checked = Boolean(data.enterDirectSave);
  summonShortcutInput.value = await invoke("get_summon_shortcut");
}

function shortcutFromEvent(event) {
  const modifierKeys = new Set(["Control", "Alt", "Shift", "Meta"]);
  if (modifierKeys.has(event.key)) return null;

  const parts = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  if (!parts.length) return null;

  const keyNames = {
    " ": "Space",
    ArrowUp: "ArrowUp",
    ArrowDown: "ArrowDown",
    ArrowLeft: "ArrowLeft",
    ArrowRight: "ArrowRight",
    Escape: "Escape",
    Enter: "Enter",
    Tab: "Tab",
  };
  const key = keyNames[event.key]
    || (event.key.length === 1 ? event.key.toUpperCase() : event.key);
  parts.push(key);
  return parts.join("+");
}

summonShortcutInput.addEventListener("keydown", async (event) => {
  event.preventDefault();
  event.stopPropagation();
  const shortcut = shortcutFromEvent(event);
  if (!shortcut) {
    shortcutStatus.textContent = "请使用至少一个修饰键和一个普通按键。";
    return;
  }

  shortcutStatus.textContent = "正在注册...";
  try {
    await invoke("set_summon_shortcut", { shortcut });
    summonShortcutInput.value = shortcut;
    shortcutStatus.textContent = "召唤快捷键已保存。";
  } catch (error) {
    shortcutStatus.textContent = String(error);
  }
});

async function loadSticky() {
  if (stickyLoaded && stickyModeLoaded) return;
  try {
    if (!stickyLoaded) {
      stickyEditor.value = await invoke("get_sticky_note");
      stickyLoaded = true;
    }
    const stickyMode = await invoke("get_sticky_mode");
    setStickyModeButtons(stickyMode === "edge" ? "edge" : "free");
    stickyModeLoaded = true;
    stickyLoaded = true;
    stickySettingsStatus.textContent = "便签自动保存到本地。";
  } catch (error) {
    stickySettingsStatus.textContent = String(error);
    stickySettingsStatus.className = "status error";
  }
}

stickyEditor.addEventListener("input", () => {
  clearTimeout(stickySaveTimer);
  stickySettingsStatus.textContent = "正在保存...";
  stickySaveTimer = setTimeout(async () => {
    try {
      await invoke("save_sticky_note", { content: stickyEditor.value });
      stickySettingsStatus.textContent = "已保存。";
    } catch (error) {
      stickySettingsStatus.textContent = String(error);
      stickySettingsStatus.className = "status error";
    }
  }, 400);
});

stickyModeButtons.forEach((button) => {
  button.onclick = async () => {
    const mode = button.dataset.mode;
    try {
      await invoke("set_sticky_mode", { mode });
      setStickyModeButtons(mode);
      stickySettingsStatus.textContent = mode === "edge"
        ? "已切换到吸边隐藏。"
        : "已切换到自由拖动。";
    } catch (error) {
      stickySettingsStatus.textContent = String(error);
      stickySettingsStatus.className = "status error";
    }
  };
});

sectionTabs.forEach((tab) => {
  tab.onclick = () => showSection(tab.dataset.section);
});

document.querySelector("#back").onclick = goBack;
document.querySelector("#closeDetails").onclick = goBack;
document.querySelector("#refresh").onclick = load;
document.querySelector("#search").oninput = load;
document.querySelector("#openSticky").onclick = () => invoke("open_sticky_note");
document.querySelector("#windowOpacity").oninput = (event) => {
  document.querySelector("#opacityValue").textContent =
    `${Math.round(Number(event.target.value) * 100)}%`;
};

document.querySelector("#saveSettings").onclick = async () => {
  await runAction("Saving settings...", async () => {
    await invoke("save_settings", {
      pageId: document.querySelector("#pageId").value.trim(),
      token: document.querySelector("#token").value,
      autostart: document.querySelector("#autostart").checked,
      windowColor: document.querySelector("#windowColor").value,
      windowOpacity: document.querySelector("#windowOpacity").value,
      moreTransparent: document.querySelector("#moreTransparent").checked,
      inputTransparent: document.querySelector("#inputTransparent").checked,
      enterDirectSave: document.querySelector("#enterDirectSave").checked,
    });
    document.querySelector("#settingsStatus").textContent = "已保存";
  });
};

showNotice("Details ready.");
loadSettings();
listen("records-changed", () => {
  if (!sections.records.hidden) load();
});
listen("sticky-mode-changed", (event) => {
  setStickyModeButtons(event.payload?.mode === "edge" ? "edge" : "free");
});

listen("update-available", (event) => {
  if (event.payload?.version) {
    showAvailableUpdate(event.payload.version);
  }
});

listen("summon-floating-bar", async () => {
  sessionStorage.setItem("focusQuickInput", "1");
  const actionsExpanded = sessionStorage.getItem("quickActionsExpanded") === "1";
  await invoke("set_details_mode", { enabled: false, actionsExpanded });
  window.location.href = "index.html";
});
setInterval(() => {
  if (!sections.records.hidden) load();
}, 3000);
