const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  bots: [],
  selected: null,           // bot id, or "__new__" for an unsaved new bot
  view: "empty",            // empty | dashboard
  currentTab: "dashboard-tab",
  currentGuild: null,
  guildRoles: [],
  selectedMember: null,
  logs: {},
};

const $ = (id) => document.getElementById(id);

function emptyBot() {
  return {
    id: null,
    name: "",
    token: "",
    xai_api_key: "",
    model: "grok-4.3",
    persona: "",
    ai_enabled: true,
    history_size: 10,
    running: false,
  };
}

async function refreshBots() {
  state.bots = await invoke("list_bots");
  renderBotList();
}

function currentBot() {
  if (state.selected === "__new__") return state._draftNew || emptyBot();
  return state.bots.find((b) => b.id === state.selected) || null;
}

function renderBotList() {
  const ul = $("bot-list");
  ul.innerHTML = "";
  for (const b of state.bots) {
    const li = document.createElement("li");
    li.textContent = b.name || "(unnamed)";
    if (state.selected === b.id) li.classList.add("active");
    const dot = document.createElement("span");
    dot.className = "dot" + (b.running ? " on" : "");
    li.appendChild(dot);
    li.onclick = () => selectBot(b.id);
    ul.appendChild(li);
  }
  if (state.selected === "__new__") {
    const li = document.createElement("li");
    li.textContent = "(new bot)";
    li.classList.add("active");
    ul.appendChild(li);
  }
}

function showView(v) {
  state.view = v;
  for (const id of ["empty", "dashboard"]) {
    $(id).classList.toggle("hidden", id !== v);
  }
}

function selectBot(id) {
  state.selected = id;
  renderBotList();
  openDashboard();
  switchTab("dashboard-tab");
}

function newBot() {
  state._draftNew = emptyBot();
  state.selected = "__new__";
  renderBotList();
  openDashboard();
  switchTab("settings-tab");
}

function openDashboard() {
  const b = currentBot();
  if (!b) { showView("empty"); return; }
  $("dash-name").textContent = b.name || "(unnamed bot)";
  const running = !!b.running;
  $("dash-status").className = "status " + (running ? "online" : "offline");
  $("dash-status").textContent = running ? "online" : "offline";

  $("start-btn").disabled = state.selected === "__new__" || running;
  $("stop-btn").disabled = !running;

  // Dashboard stats
  $("ds-status").textContent = running ? "Online" : "Offline";
  $("ds-ai").textContent = b.ai_enabled ? "Yes" : "No";
  $("ds-model").textContent = b.model || "—";
  $("ds-guilds").textContent = "—";
  $("ds-hint").textContent = state.selected === "__new__"
    ? "Save the bot in the Settings tab first, then press Start."
    : (running ? "" : "Bot is offline. Press Start to bring it online.");

  // Settings tab
  $("f-name").value = b.name || "";
  $("f-token").value = b.token || "";

  // AI tab
  $("ai-xai").value = b.xai_api_key || "";
  const wantModel = b.model || "grok-4.3";
  if (![...$("ai-model").options].some((o) => o.value === wantModel)) {
    const opt = document.createElement("option");
    opt.value = wantModel; opt.textContent = wantModel;
    $("ai-model").appendChild(opt);
  }
  $("ai-model").value = wantModel;
  $("ai-history").value = b.history_size || 10;
  $("ai-enabled").checked = b.ai_enabled ?? true;
  $("ai-persona").value = b.persona || "";

  // Voice tab
  $("voice-enabled").checked = b.voice_enabled ?? true;
  $("voice-name").value = b.voice || "eve";
  loadVoices(b.xai_api_key);

  // Reset transient lists
  $("guild-list").innerHTML = "";
  $("member-list").innerHTML = "";
  $("console-log").textContent = (state.logs[b.id] || []).join("\n");

  showView("dashboard");

  if (running) {
    loadGuilds().then(() => {
      $("ds-guilds").textContent = $("guild-list").children.length;
    });
  }
}

async function saveSettings() {
  const b = currentBot();
  if (!b) return;
  const name = $("f-name").value.trim();
  const token = $("f-token").value.trim();
  if (!name || !token) { alert("Name and token required."); return; }
  const input = {
    id: state.selected === "__new__" ? null : state.selected,
    name,
    token,
    xai_api_key: b.xai_api_key || "",
    model: b.model || "grok-4.3",
    persona: b.persona || "",
    ai_enabled: b.ai_enabled ?? true,
    history_size: b.history_size || 10,
  };
  const saved = await invoke("save_bot", { input });
  state.selected = saved.id;
  state._draftNew = null;
  await refreshBots();
  openDashboard();
}

async function saveAISettings() {
  if (state.selected === "__new__") { alert("Save the bot in Settings tab first."); return; }
  const b = currentBot();
  if (!b) return;
  const input = {
    id: state.selected,
    name: b.name,
    token: b.token,
    xai_api_key: $("ai-xai").value.trim(),
    model: $("ai-model").value.trim(),
    persona: $("ai-persona").value,
    ai_enabled: $("ai-enabled").checked,
    history_size: parseInt($("ai-history").value, 10) || 10,
  };
  await invoke("save_bot", { input });
  await refreshBots();
  openDashboard();
  switchTab("ai-tab");
}

async function loadVoices(apiKey) {
  const key = (apiKey || "").trim();
  const dl = $("voice-list");
  const status = $("voice-list-status");
  if (!key) { status.textContent = "Set an xAI API key (AI Settings) to load voices."; return; }
  status.textContent = "Loading voices…";
  try {
    const voices = await invoke("list_voices", { apiKey: key });
    dl.innerHTML = "";
    for (const v of voices) {
      const opt = document.createElement("option");
      opt.value = v.id;
      opt.label = `${v.label} [${v.kind}]`;
      dl.appendChild(opt);
    }
    status.textContent = voices.length ? `${voices.length} voices available.` : "No voices returned.";
  } catch (e) {
    status.textContent = `Could not load voices: ${e}`;
  }
}

async function saveVoiceSettings() {
  if (state.selected === "__new__") { alert("Save the bot in Settings tab first."); return; }
  const b = currentBot();
  if (!b) return;
  const input = {
    id: state.selected,
    name: b.name,
    token: b.token,
    voice_enabled: $("voice-enabled").checked,
    voice: $("voice-name").value,
  };
  await invoke("save_bot", { input });
  await refreshBots();
  openDashboard();
  switchTab("voice-tab");
}

async function deleteBot() {
  if (state.selected === "__new__") {
    state._draftNew = null;
    state.selected = null;
    renderBotList();
    showView("empty");
    return;
  }
  if (!state.selected) return;
  if (!confirm("Delete this bot?")) return;
  await invoke("delete_bot", { id: state.selected });
  state.selected = null;
  await refreshBots();
  showView("empty");
}

async function loadGuilds() {
  try {
    const guilds = await invoke("list_guilds", { id: state.selected });
    const ul = $("guild-list");
    ul.innerHTML = "";
    for (const g of guilds) {
      const li = document.createElement("li");
      li.textContent = `${g.name} (${g.member_count})`;
      li.onclick = (ev) => selectGuild(g, ev.currentTarget);
      ul.appendChild(li);
    }
    $("servers-hint").style.display = guilds.length ? "none" : "";
  } catch (e) { appendLog(state.selected, "error", String(e)); }
}

async function selectGuild(g, target) {
  state.currentGuild = g.id;
  $("members-title").textContent = `Members — ${g.name}`;
  for (const li of $("guild-list").children) li.classList.remove("active");
  if (target) target.classList.add("active");
  $("members-hint").style.display = "none";
  await refreshMembers();
  state.guildRoles = await invoke("list_guild_roles", { id: state.selected, guildId: g.id });
  switchTab("members-tab");
}

async function refreshMembers() {
  if (!state.currentGuild) return;
  try {
    const members = await invoke("list_guild_members", { id: state.selected, guildId: state.currentGuild });
    const filter = $("member-search").value.toLowerCase();
    const ul = $("member-list");
    ul.innerHTML = "";
    for (const m of members) {
      if (filter && !(m.username + (m.nick || "")).toLowerCase().includes(filter)) continue;
      const li = document.createElement("li");
      li.textContent = (m.nick ? `${m.nick} (${m.username})` : m.username) + ` — ${m.user_id}`;
      li.onclick = () => openMember(m);
      ul.appendChild(li);
    }
  } catch (e) { appendLog(state.selected, "error", String(e)); }
}

function openMember(m) {
  state.selectedMember = m;
  $("member-modal-title").textContent = m.username;
  $("mm-user-id").textContent = m.user_id;
  const memberRoleIds = new Set(m.roles.map((r) => r.id));
  const wrap = $("mm-roles");
  wrap.innerHTML = "";
  for (const r of state.guildRoles) {
    if (r.name === "@everyone") continue;
    const chip = document.createElement("span");
    chip.className = "role-chip" + (memberRoleIds.has(r.id) ? " has" : "");
    chip.textContent = r.name;
    chip.onclick = async () => {
      try {
        const cmd = memberRoleIds.has(r.id) ? "remove_role" : "add_role";
        await invoke(cmd, { id: state.selected, guildId: state.currentGuild, userId: m.user_id, roleId: r.id });
        await refreshMembers();
        const updated = (await invoke("list_guild_members", { id: state.selected, guildId: state.currentGuild })).find((x) => x.user_id === m.user_id);
        if (updated) openMember(updated);
      } catch (e) { alert(e); }
    };
    wrap.appendChild(chip);
  }
  $("member-modal").classList.remove("hidden");
}

async function memberAction(act) {
  const m = state.selectedMember;
  if (!m) return;
  try {
    if (act === "kick") await invoke("kick", { id: state.selected, guildId: state.currentGuild, userId: m.user_id });
    if (act === "ban") {
      if (!confirm(`Ban ${m.username}?`)) return;
      await invoke("ban", { id: state.selected, guildId: state.currentGuild, userId: m.user_id });
    }
    if (act === "timeout") {
      const s = parseInt($("mm-timeout").value, 10) || 60;
      await invoke("timeout", { id: state.selected, guildId: state.currentGuild, userId: m.user_id, seconds: s });
    }
    $("member-modal").classList.add("hidden");
    await refreshMembers();
  } catch (e) { alert(e); }
}

function appendLog(botId, level, msg) {
  state.logs[botId] = state.logs[botId] || [];
  const line = `[${new Date().toLocaleTimeString()}] [${level}] ${msg}`;
  state.logs[botId].push(line);
  if (state.logs[botId].length > 500) state.logs[botId].shift();
  if (state.selected === botId && state.view === "dashboard") {
    $("console-log").textContent = state.logs[botId].join("\n");
    $("console-log").scrollTop = $("console-log").scrollHeight;
  }
}

function switchTab(tabId) {
  state.currentTab = tabId;
  document.querySelectorAll(".tab-content").forEach((t) => { t.classList.remove("active"); t.classList.add("hidden"); });
  document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
  const content = $(tabId);
  if (content) { content.classList.add("active"); content.classList.remove("hidden"); }
  document.querySelector(`[data-tab="${tabId}"]`)?.classList.add("active");
  if (tabId === "console-tab" && state.selected) {
    $("console-log").textContent = (state.logs[state.selected] || []).join("\n");
  }
}

// Event wiring
$("new-bot-btn").onclick = newBot;
$("save-btn").onclick = saveSettings;
$("delete-btn").onclick = deleteBot;
$("start-btn").onclick = async () => {
  if (state.selected === "__new__") { alert("Save the bot first."); return; }
  try { await invoke("start_bot_cmd", { id: state.selected }); await refreshBots(); openDashboard(); } catch (e) { alert(e); }
};
$("stop-btn").onclick = async () => {
  try { await invoke("stop_bot_cmd", { id: state.selected }); await refreshBots(); openDashboard(); } catch (e) { alert(e); }
};
$("refresh-members").onclick = refreshMembers;
$("member-search").oninput = refreshMembers;
$("mm-close").onclick = () => $("member-modal").classList.add("hidden");
$("ai-save-btn").onclick = saveAISettings;
$("voice-save-btn").onclick = saveVoiceSettings;
$("voice-refresh-btn").onclick = () => { const b = currentBot(); loadVoices(b?.xai_api_key); };
document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.onclick = () => switchTab(btn.dataset.tab);
});
document.querySelectorAll("#member-modal [data-act]").forEach((b) => b.onclick = () => memberAction(b.dataset.act));

listen("bot-log", (ev) => {
  const { bot_id, level, msg } = ev.payload;
  appendLog(bot_id, level, msg);
});
listen("bot-status", async (ev) => {
  await refreshBots();
  if (state.selected === ev.payload.bot_id) openDashboard();
});

refreshBots();
