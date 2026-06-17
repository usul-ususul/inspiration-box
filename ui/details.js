const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
const convertFileSrc = window.__TAURI__.core.convertFileSrc;
const recordsEl = document.querySelector("#records");
const noticeEl = document.querySelector("#detailNotice");

const statusName = {
  pending: "Pending",
  failed: "Failed",
  synced: "Synced",
};

function showNotice(message) {
  if (noticeEl) noticeEl.textContent = message;
}

function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text || "";
  return div.innerHTML;
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
  await invoke("set_details_mode", { enabled: false });
  window.location.href = "index.html";
}

async function load() {
  try {
    const records = await invoke("list_records", {
      search: document.querySelector("#search").value,
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
        ? `<button class="secondary" type="button" data-action="retry">&#37325;&#35797;&#21516;&#27493;</button>`
        : "";

      return `<article class="record" data-id="${record.id}">
        <div class="meta">
          <span>${new Date(record.created_at).toLocaleString()}</span>
          <span>${statusName[record.status] || record.status}</span>
          <span>${actionText}</span>
        </div>
        ${record.category ? `<span class="category">${escapeHtml(record.category)}</span>` : ""}
        <div class="content">${escapeHtml(record.content)}</div>
        ${imageHtml}
        ${record.error ? `<p class="error">${escapeHtml(record.error)}</p>` : ""}
        <div class="record-actions">
          <button class="secondary" type="button" data-action="edit">&#32534;&#36753;</button>
          <button class="secondary" type="button" data-action="delete">&#21024;&#38500;</button>
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
      <button class="secondary" type="button" data-action="save-edit">&#20445;&#23384;</button>
      <button class="secondary" type="button" data-action="cancel-edit">&#21462;&#28040;</button>
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
}

document.querySelector("#back").onclick = goBack;
document.querySelector("#closeDetails").onclick = goBack;
document.querySelector("#refresh").onclick = load;
document.querySelector("#search").oninput = load;

document.querySelector("#saveSettings").onclick = async () => {
  await runAction("Saving settings...", async () => {
    await invoke("save_settings", {
      pageId: document.querySelector("#pageId").value.trim(),
      token: document.querySelector("#token").value,
      autostart: document.querySelector("#autostart").checked,
    });
    document.querySelector("#settingsStatus").textContent = " Saved";
  });
};

showNotice("Details ready.");
load();
loadSettings();
listen("records-changed", load);
setInterval(load, 3000);
