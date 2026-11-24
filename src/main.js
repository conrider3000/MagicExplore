const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const ENTER_DOUBLE_WINDOW_MS = 280;
const BASE_BAR_HEIGHT = 64;

const searchView = document.getElementById("search-view");
const settingsView = document.getElementById("settings-view");
const searchInput = document.getElementById("search-input");
const hotkeyPill = document.getElementById("hotkey-pill");
const openSettingsBtn = document.getElementById("open-settings-btn");
const backToSearchBtn = document.getElementById("back-to-search-btn");

const resultsContainer = document.getElementById("results-container");
const resultsList = document.getElementById("results-list");
const fallbackContainer = document.getElementById("fallback-container");
const explorerQueryPreview = document.getElementById("explorer-query-preview");
const webQueryPreview = document.getElementById("web-query-preview");
const btnSearchExplorer = document.getElementById("btn-search-explorer");
const btnSearchWeb = document.getElementById("btn-search-web");
const badge1xEnter = document.getElementById("badge-1x-enter");

const toggleAutostart = document.getElementById("toggle-autostart");
const toggleOpaqueBackdrop = document.getElementById("toggle-opaque-backdrop");
const toggleAdminToast = document.getElementById("toggle-admin-toast");
const settingsHotkeyDisplay = document.getElementById("settings-hotkey-display");
const hotkeyStatusDesc = document.getElementById("hotkey-status-desc");
const recordHotkeyBtn = document.getElementById("record-hotkey-btn");
const refreshAppsBtn = document.getElementById("refresh-apps-btn");
const appCountDesc = document.getElementById("app-count-desc");

const conflictModal = document.getElementById("conflict-modal");
const conflictTitle = document.getElementById("conflict-title");
const conflictMessage = document.getElementById("conflict-message");
const conflictTip = document.getElementById("conflict-tip");
const conflictCancelBtn = document.getElementById("conflict-cancel-btn");
const conflictConfirmBtn = document.getElementById("conflict-confirm-btn");

let allApps = [];
let filteredApps = [];
let selectedIndex = 0;
let enterTimer = null;
let currentView = "search";
let isRecordingHotkey = false;
let pendingConflictAction = null;
let lastRequestedHeight = 0;

function normalizeText(str) {
  return (str || "")
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .toLowerCase()
    .trim();
}

function escapeHtml(text) {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function setWindowHeight(height, force = false) {
  const rounded = Math.round(height);
  if (!force && rounded === lastRequestedHeight) return;
  lastRequestedHeight = rounded;
  invoke("set_window_height", { height: rounded }).catch(() => {});
}

function updateDynamicHeight(force = false) {
  if (!conflictModal.classList.contains("hidden")) {
    setWindowHeight(315, force);
    return;
  }
  if (currentView === "settings") {
    setWindowHeight(410, force);
    return;
  }
  if (!resultsContainer.classList.contains("hidden")) {
    const visibleCount = Math.min(filteredApps.length, 6);
    const listHeight = 16 + visibleCount * 50 - 2;
    setWindowHeight(BASE_BAR_HEIGHT + 1 + listHeight, force);
    return;
  }
  if (!fallbackContainer.classList.contains("hidden")) {
    setWindowHeight(BASE_BAR_HEIGHT + 1 + 124, force);
    return;
  }
  setWindowHeight(BASE_BAR_HEIGHT, force);
}

function scoreApp(appNameNorm, queryNorm) {
  if (!queryNorm) return -1;
  if (appNameNorm === queryNorm) return 1000;
  if (appNameNorm.startsWith(queryNorm)) return 800 - appNameNorm.length * 0.01;

  const words = appNameNorm.split(/[\s\-_.:/()]+/).filter(Boolean);
  for (let i = 0; i < words.length; i++) {
    if (words[i].startsWith(queryNorm)) return 620 - i * 10;
  }

  const idx = appNameNorm.indexOf(queryNorm);
  if (idx !== -1) return 420 - idx;

  if (queryNorm.length >= 2 && words.length >= queryNorm.length) {
    const initials = words.map((w) => w[0]).join("");
    if (initials.startsWith(queryNorm)) return 300;
  }
  return -1;
}

function highlightMatch(name, rawQuery) {
  const qNorm = normalizeText(rawQuery);
  if (!qNorm) return escapeHtml(name);
  const nameNorm = normalizeText(name);
  const idx = nameNorm.indexOf(qNorm);
  if (idx === -1 || nameNorm.length !== name.length) {
    return escapeHtml(name);
  }
  const before = escapeHtml(name.slice(0, idx));
  const match = escapeHtml(name.slice(idx, idx + qNorm.length));
  const after = escapeHtml(name.slice(idx + qNorm.length));
  return `${before}<mark>${match}</mark>${after}`;
}

function makeFallbackAvatar(name) {
  const clean = (name || "?").trim();
  const ch = clean.charAt(0).toUpperCase() || "?";
  const hue = (clean.charCodeAt(0) * 37 + clean.length * 19) % 360;
  return `<div class="app-avatar" style="background: linear-gradient(145deg, hsla(${hue}, 70%, 55%, 0.45), hsla(${hue}, 70%, 35%, 0.25));">${escapeHtml(ch)}</div>`;
}

function clearEnterTimer() {
  if (enterTimer !== null) {
    clearTimeout(enterTimer);
    enterTimer = null;
  }
  badge1xEnter.classList.remove("pending");
}

function renderResults(forceResize = false) {
  const rawQuery = searchInput.value;
  const queryNorm = normalizeText(rawQuery);

  if (!queryNorm) {
    filteredApps = [];
    selectedIndex = 0;
    resultsList.innerHTML = "";
    resultsContainer.classList.add("hidden");
    fallbackContainer.classList.add("hidden");
    updateDynamicHeight(forceResize);
    return;
  }

  const scored = [];
  for (const app of allApps) {
    const s = scoreApp(app._normName, queryNorm);
    if (s > 0) scored.push({ app, score: s });
  }
  scored.sort((a, b) => b.score - a.score || a.app.name.localeCompare(b.app.name));
  filteredApps = scored.slice(0, 18).map((x) => x.app);
  selectedIndex = 0;

  if (filteredApps.length > 0) {
    fallbackContainer.classList.add("hidden");
    resultsContainer.classList.remove("hidden");
    resultsList.innerHTML = filteredApps
      .map((app, i) => {
        const avatar = app.icon_data_url
          ? `<div class="app-avatar"><img src="${app.icon_data_url}" alt="" /></div>`
          : makeFallbackAvatar(app.name);
        return `
          <li class="result-item ${i === selectedIndex ? "selected" : ""}" data-index="${i}" role="option">
            ${avatar}
            <div class="result-meta">
              <div class="result-name">${highlightMatch(app.name, rawQuery)}</div>
              <div class="result-source">${escapeHtml(app.source)}</div>
            </div>
            <span class="result-hint">↵ Abrir</span>
          </li>
        `;
      })
      .join("");
  } else {
    resultsList.innerHTML = "";
    resultsContainer.classList.add("hidden");
    fallbackContainer.classList.remove("hidden");
    const preview = rawQuery.trim().length > 26
      ? `"${rawQuery.trim().slice(0, 26)}…"`
      : `"${rawQuery.trim()}"`;
    explorerQueryPreview.textContent = preview;
    webQueryPreview.textContent = preview;
  }

  updateDynamicHeight(forceResize);
}

function updateSelectedRow() {
  const items = resultsList.querySelectorAll(".result-item");
  items.forEach((el, idx) => {
    el.classList.toggle("selected", idx === selectedIndex);
  });
  const active = items[selectedIndex];
  if (active) {
    active.scrollIntoView({ block: "nearest" });
  }
}

async function launchSelectedApp(index = selectedIndex) {
  const app = filteredApps[index];
  if (!app) return;
  clearEnterTimer();
  await invoke("launch_app", { appId: app.id });
}

async function runExplorerSearch() {
  const q = searchInput.value.trim();
  if (!q) return;
  clearEnterTimer();
  await invoke("search_explorer", { query: q });
}

async function runWebSearch() {
  const q = searchInput.value.trim();
  if (!q) return;
  clearEnterTimer();
  await invoke("search_web", { query: q });
}

function switchView(viewName) {
  currentView = viewName;
  clearEnterTimer();
  if (isRecordingHotkey) {
    stopHotkeyCaptureUI();
    invoke("stop_hotkey_capture").catch(() => {});
  }

  if (viewName === "settings") {
    searchView.classList.add("hidden");
    settingsView.classList.remove("hidden");
    refreshState();
  } else {
    settingsView.classList.add("hidden");
    searchView.classList.remove("hidden");
    setTimeout(() => searchInput.focus(), 10);
  }
  updateDynamicHeight(true);
}

function showConflictModal({ title, message, tip, confirmLabel, onConfirm, onCancel }) {
  conflictTitle.textContent = title || "Substituir atalho do Windows?";
  conflictMessage.textContent = message;
  conflictTip.textContent = tip || "";
  conflictTip.classList.toggle("hidden", !tip);
  conflictConfirmBtn.textContent = confirmLabel || "Autorizar e Ativar";
  pendingConflictAction = { onConfirm, onCancel };
  conflictModal.classList.remove("hidden");
  updateDynamicHeight(true);
}

function hideConflictModal() {
  conflictModal.classList.add("hidden");
  pendingConflictAction = null;
  updateDynamicHeight(true);
}

function startHotkeyCaptureUI() {
  isRecordingHotkey = true;
  settingsHotkeyDisplay.textContent = "Pressione o atalho...";
  settingsHotkeyDisplay.classList.add("recording");
  recordHotkeyBtn.textContent = "Cancelar";
  hotkeyStatusDesc.textContent = "Pressione a combinação desejada (ex: Win + Space, Alt + Space) ou Esc";
  invoke("start_hotkey_capture").catch(() => stopHotkeyCaptureUI());
}

function stopHotkeyCaptureUI() {
  isRecordingHotkey = false;
  settingsHotkeyDisplay.classList.remove("recording");
  recordHotkeyBtn.textContent = "Alterar";
  hotkeyStatusDesc.textContent = "Ativo agora — clique para escolher outra combinação";
}

async function refreshState() {
  try {
    const state = await invoke("get_state");
    hotkeyPill.textContent = state.hotkey_display;
    if (!isRecordingHotkey) {
      settingsHotkeyDisplay.textContent = state.hotkey_display;
    }
    toggleAutostart.checked = state.config.autostart;
    toggleOpaqueBackdrop.checked = Boolean(state.config.opaque_backdrop);
    toggleAdminToast.checked = state.config.show_admin_toast;
    appCountDesc.textContent = `${state.app_count} aplicativos indexados em memória`;

    if (state.needs_consent) {
      showConflictModal({
        title: "Autorizar substituição de Win + Space?",
        message:
          "O Magic Explore usa o atalho Win + Space para abrir a barra de busca, substituindo a função nativa do Windows de alternar idioma/layout do teclado.",
        tip: "Você ainda poderá trocar o idioma pela barra de tarefas ou pressionando Alt + Shift.",
        confirmLabel: "Autorizar Win + Space",
        onConfirm: async () => {
          await invoke("respond_initial_consent", { authorizeWinSpace: true });
          await refreshState();
          switchView("search");
        },
        onCancel: async () => {
          await invoke("respond_initial_consent", { authorizeWinSpace: false });
          await refreshState();
          switchView("search");
        },
      });
    }
  } catch (err) {
    console.error("Erro ao carregar estado:", err);
  }
}

async function loadApps() {
  try {
    const apps = await invoke("get_apps");
    allApps = apps.map((a) => ({
      ...a,
      _normName: normalizeText(a.name),
    }));
    renderResults();
  } catch (err) {
    console.error("Erro ao carregar aplicativos:", err);
  }
}

searchInput.addEventListener("input", () => {
  clearEnterTimer();
  renderResults();
});

searchInput.addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown") {
    if (filteredApps.length > 0) {
      e.preventDefault();
      selectedIndex = (selectedIndex + 1) % filteredApps.length;
      updateSelectedRow();
    }
    return;
  }

  if (e.key === "ArrowUp") {
    if (filteredApps.length > 0) {
      e.preventDefault();
      selectedIndex = (selectedIndex - 1 + filteredApps.length) % filteredApps.length;
      updateSelectedRow();
    }
    return;
  }

  if (e.key === "Enter") {
    e.preventDefault();
    const q = searchInput.value.trim();
    if (!q) return;

    if (q.toLowerCase() === ":config") {
      searchInput.value = "";
      renderResults();
      switchView("settings");
      return;
    }

    if (filteredApps.length > 0) {
      launchSelectedApp(selectedIndex);
      return;
    }

    if (enterTimer !== null) {
      runWebSearch();
    } else {
      badge1xEnter.classList.add("pending");
      enterTimer = setTimeout(() => {
        runExplorerSearch();
      }, ENTER_DOUBLE_WINDOW_MS);
    }
  }
});

resultsList.addEventListener("click", (e) => {
  const row = e.target.closest(".result-item");
  if (!row) return;
  const idx = Number(row.dataset.index);
  launchSelectedApp(idx);
});

resultsList.addEventListener("mousemove", (e) => {
  const row = e.target.closest(".result-item");
  if (!row) return;
  const idx = Number(row.dataset.index);
  if (idx !== selectedIndex) {
    selectedIndex = idx;
    updateSelectedRow();
  }
});

btnSearchExplorer.addEventListener("click", runExplorerSearch);
btnSearchWeb.addEventListener("click", runWebSearch);

openSettingsBtn.addEventListener("click", () => switchView("settings"));
backToSearchBtn.addEventListener("click", () => switchView("search"));

toggleAutostart.addEventListener("change", async () => {
  try {
    await invoke("set_autostart", { enabled: toggleAutostart.checked });
  } catch (err) {
    toggleAutostart.checked = !toggleAutostart.checked;
  }
});

toggleOpaqueBackdrop.addEventListener("change", async () => {
  try {
    await invoke("set_opaque_backdrop", { enabled: toggleOpaqueBackdrop.checked });
  } catch (err) {
    toggleOpaqueBackdrop.checked = !toggleOpaqueBackdrop.checked;
  }
});

toggleAdminToast.addEventListener("change", async () => {
  try {
    await invoke("set_show_admin_toast", { enabled: toggleAdminToast.checked });
  } catch (err) {
    toggleAdminToast.checked = !toggleAdminToast.checked;
  }
});

recordHotkeyBtn.addEventListener("click", () => {
  if (isRecordingHotkey) {
    stopHotkeyCaptureUI();
    invoke("stop_hotkey_capture").catch(() => {});
    refreshState();
  } else {
    startHotkeyCaptureUI();
  }
});

refreshAppsBtn.addEventListener("click", async () => {
  refreshAppsBtn.disabled = true;
  refreshAppsBtn.textContent = "Atualizando...";
  try {
    const count = await invoke("refresh_apps");
    appCountDesc.textContent = `${count} aplicativos indexados em memória`;
    await loadApps();
  } finally {
    refreshAppsBtn.disabled = false;
    refreshAppsBtn.textContent = "Atualizar";
  }
});

conflictCancelBtn.addEventListener("click", async () => {
  const action = pendingConflictAction;
  hideConflictModal();
  if (action && action.onCancel) await action.onCancel();
});

conflictConfirmBtn.addEventListener("click", async () => {
  const action = pendingConflictAction;
  hideConflictModal();
  if (action && action.onConfirm) await action.onConfirm();
});

window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") {
    e.preventDefault();
    if (isRecordingHotkey) {
      stopHotkeyCaptureUI();
      invoke("stop_hotkey_capture").catch(() => {});
      refreshState();
      return;
    }
    if (!conflictModal.classList.contains("hidden")) {
      hideConflictModal();
    }
    clearEnterTimer();
    invoke("hide_main_window").catch(() => {});
  }
}, true);

listen("bar-opened", () => {
  clearEnterTimer();
  if (!conflictModal.classList.contains("hidden")) return;
  currentView = "search";
  settingsView.classList.add("hidden");
  searchView.classList.remove("hidden");
  searchInput.value = "";
  renderResults(true);
  setTimeout(() => {
    searchInput.focus();
  }, 15);
});

listen("open-settings", () => {
  switchView("settings");
});

listen("apps-updated", () => {
  loadApps();
  refreshState();
});

listen("hotkey-capture-cancelled", () => {
  stopHotkeyCaptureUI();
  refreshState();
});

listen("hotkey-captured", async (event) => {
  const payload = event.payload;
  stopHotkeyCaptureUI();

  if (payload.blocked) {
    showConflictModal({
      title: "Atalho bloqueado pelo Windows",
      message: payload.conflict_reason,
      tip: "Escolha uma combinação diferente (ex: Win + Space, Alt + Space ou Ctrl + Space).",
      confirmLabel: "Entendi",
      onConfirm: async () => {
        await refreshState();
      },
      onCancel: async () => {
        await refreshState();
      },
    });
    return;
  }

  if (payload.conflict_reason) {
    showConflictModal({
      title: `Substituir "${payload.display}" do Windows?`,
      message: `O atalho ${payload.display} é usado nativamente pelo Windows para: ${payload.conflict_reason}. Deseja autorizar o Magic Explore a assumir esta tecla?`,
      tip:
        payload.display === "Win + Space"
          ? "Dica: você continuará podendo trocar o idioma pelo Alt + Shift ou pela barra de tarefas."
          : "Você pode reverter para outro atalho a qualquer momento nas Configurações.",
      confirmLabel: "Autorizar e Salvar",
      onConfirm: async () => {
        await invoke("apply_hotkey", { hotkey: payload.hotkey, authorized: true });
        await refreshState();
      },
      onCancel: async () => {
        await refreshState();
      },
    });
    return;
  }

  await invoke("apply_hotkey", { hotkey: payload.hotkey, authorized: true });
  await refreshState();
});

(async function init() {
  await refreshState();
  await loadApps();
  updateDynamicHeight(true);
})();
