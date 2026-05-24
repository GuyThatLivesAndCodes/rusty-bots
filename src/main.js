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
    automod: {
      enabled: false,
      rules: [],
      ignored_channels: [],
      ignored_roles: [],
      whitelist_users: [],
      log_channel: null,
      advanced_detection: {
        enable_spaced_variant: true,
        enable_special_char_variant: true,
        enable_acronym_detection: true,
        enable_cross_message_detection: true,
        cross_message_window_secs: 60,
      },
    },
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

  // Reset transient lists
  $("guild-list").innerHTML = "";
  $("member-list").innerHTML = "";
  $("console-log").textContent = (state.logs[b.id] || []).join("\n");

  // Auto-Mod tab
  loadAutoModConfig(b);

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

async function loadAutoModConfig(b) {
  if (state.selected === "__new__" || !state.selected) {
    const config = b.automod || emptyBot().automod;
    populateAutoModUI(config);
    return;
  }
  try {
    const config = await invoke("get_automod_config", { id: state.selected });
    populateAutoModUI(config);
  } catch (e) {
    console.error("Failed to load auto-mod config:", e);
    const config = b.automod || emptyBot().automod;
    populateAutoModUI(config);
  }
}

function populateAutoModUI(config) {
  $("automod-enabled").checked = config.enabled || false;
  $("detect-spaced").checked = config.advanced_detection?.enable_spaced_variant ?? true;
  $("detect-special-chars").checked = config.advanced_detection?.enable_special_char_variant ?? true;
  $("detect-acronym").checked = config.advanced_detection?.enable_acronym_detection ?? true;
  $("detect-cross-message").checked = config.advanced_detection?.enable_cross_message_detection ?? true;
  $("cross-message-window").value = config.advanced_detection?.cross_message_window_secs ?? 60;
  $("ignore-channels-input").value = (config.ignored_channels || []).join(", ");
  $("ignore-roles-input").value = (config.ignored_roles || []).join(", ");
  $("whitelist-users-input").value = (config.whitelist_users || []).join(", ");
  $("log-channel-input").value = config.log_channel || "";
  renderRulesList(config.rules || []);
}

function renderRulesList(rules) {
  const list = $("rules-list");
  list.innerHTML = "";
  for (let i = 0; i < rules.length; i++) {
    const rule = rules[i];
    const li = document.createElement("li");
    li.className = "rule-item";
    li.innerHTML = `
      <div class="rule-content">
        <strong>${rule.type}</strong>
        <button class="btn-small" onclick="deleteRule(${i})">Delete</button>
      </div>
      <pre>${JSON.stringify(rule, null, 2)}</pre>
    `;
    list.appendChild(li);
  }
}

async function deleteRule(index) {
  if (state.selected === "__new__" || !state.selected) return;
  try {
    await invoke("remove_automod_rule", { id: state.selected, ruleIndex: index });
    const config = await invoke("get_automod_config", { id: state.selected });
    populateAutoModUI(config);
  } catch (e) { alert("Error deleting rule: " + e); }
}

async function addRule() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const ruleType = $("rule-type").value;
  let rule;

  if (ruleType === "BadWords") {
    const words = prompt("Enter bad words (comma-separated):") || "";
    if (!words.trim()) return;
    rule = {
      type: "BadWords",
      words: words.split(",").map(w => w.trim()),
      action: { type: "Delete" },
    };
  } else if (ruleType === "SpamDetection") {
    rule = {
      type: "SpamDetection",
      message_threshold: 5,
      time_window_secs: 10,
      action: { type: "Timeout", duration_secs: 60 },
    };
  } else if (ruleType === "Caps") {
    rule = {
      type: "Caps",
      threshold_percent: 70.0,
      action: { type: "Delete" },
    };
  } else if (ruleType === "MentionSpam") {
    rule = {
      type: "MentionSpam",
      mention_threshold: 5,
      action: { type: "Delete" },
    };
  } else if (ruleType === "LinkFilter") {
    const domains = prompt("Enter allowed domains (comma-separated):", "discord.com") || "";
    rule = {
      type: "LinkFilter",
      allowed_domains: domains.split(",").map(d => d.trim()),
      action: { type: "Delete" },
    };
  } else if (ruleType === "InviteFilter") {
    rule = {
      type: "InviteFilter",
      action: { type: "Delete" },
    };
  }

  try {
    await invoke("add_automod_rule", { id: state.selected, rule });
    const config = await invoke("get_automod_config", { id: state.selected });
    populateAutoModUI(config);
  } catch (e) { alert("Error adding rule: " + e); }
}

async function saveDetectionSettings() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const config = {
    enable_spaced_variant: $("detect-spaced").checked,
    enable_special_char_variant: $("detect-special-chars").checked,
    enable_acronym_detection: $("detect-acronym").checked,
    enable_cross_message_detection: $("detect-cross-message").checked,
    cross_message_window_secs: parseInt($("cross-message-window").value, 10) || 60,
  };
  try {
    await invoke("update_advanced_detection_config", { id: state.selected, config });
    alert("Detection settings saved!");
  } catch (e) { alert("Error saving: " + e); }
}

async function saveIgnoredChannels() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const channels = $("ignore-channels-input").value
    .split(",")
    .map(s => s.trim())
    .filter(s => s);
  try {
    await invoke("set_automod_ignored_channels", { id: state.selected, channels });
    alert("Ignored channels saved!");
  } catch (e) { alert("Error saving: " + e); }
}

async function saveIgnoredRoles() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const roles = $("ignore-roles-input").value
    .split(",")
    .map(s => s.trim())
    .filter(s => s);
  try {
    await invoke("set_automod_ignored_roles", { id: state.selected, roles });
    alert("Ignored roles saved!");
  } catch (e) { alert("Error saving: " + e); }
}

async function saveWhitelist() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const users = $("whitelist-users-input").value
    .split(",")
    .map(s => s.trim())
    .filter(s => s);
  try {
    await invoke("set_automod_whitelist", { id: state.selected, users });
    alert("Whitelist saved!");
  } catch (e) { alert("Error saving: " + e); }
}

async function saveLogChannel() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const channel = $("log-channel-input").value.trim() || null;
  try {
    await invoke("set_automod_log_channel", { id: state.selected, channel });
    alert("Log channel saved!");
  } catch (e) { alert("Error saving: " + e); }
}

async function toggleAutoMod() {
  if (state.selected === "__new__" || !state.selected) { alert("Save the bot first."); return; }
  const enabled = $("automod-enabled").checked;
  try {
    await invoke("toggle_automod", { id: state.selected, enabled });
  } catch (e) { alert("Error: " + e); }
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
$("automod-enabled").onchange = toggleAutoMod;
$("add-rule-btn").onclick = addRule;
$("save-detection-btn").onclick = saveDetectionSettings;
$("save-ignored-channels-btn").onclick = saveIgnoredChannels;
$("save-ignored-roles-btn").onclick = saveIgnoredRoles;
$("save-whitelist-btn").onclick = saveWhitelist;
$("save-log-channel-btn").onclick = saveLogChannel;
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
