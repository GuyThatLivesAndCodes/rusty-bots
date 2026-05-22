const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  bots: [],
  selected: null,
  view: "empty", // empty | editor | dashboard
  currentGuild: null,
  guildRoles: [],
  selectedMember: null,
  logs: {},
};

const $ = (id) => document.getElementById(id);

async function refreshBots() {
  state.bots = await invoke("list_bots");
  renderBotList();
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
}

function showView(v) {
  state.view = v;
  for (const id of ["empty", "editor", "dashboard"]) {
    $(id).classList.toggle("hidden", id !== v);
  }
}

function selectBot(id) {
  state.selected = id;
  const b = state.bots.find((x) => x.id === id);
  if (!b) return;
  renderBotList();
  if (b.running) openDashboard(b);
  else openEditor(b);
}

function openEditor(b) {
  $("editor-title").textContent = b ? `Edit: ${b.name}` : "New Bot";
  $("f-name").value = b?.name || "";
  $("f-token").value = b?.token || "";
  $("f-xai").value = b?.xai_api_key || "";
  $("f-model").value = b?.model || "grok-4.3";
  if (![...$("f-model").options].some((o) => o.value === $("f-model").value)) {
    const opt = document.createElement("option");
    opt.value = b.model; opt.textContent = b.model;
    $("f-model").appendChild(opt); $("f-model").value = b.model;
  }
  $("f-history").value = b?.history_size ?? 10;
  $("f-ai").checked = b?.ai_enabled ?? true;
  $("f-persona").value = b?.persona || "";
  $("delete-btn").style.display = b ? "" : "none";
  showView("editor");
}

async function saveBot() {
  const input = {
    id: state.selected || null,
    name: $("f-name").value.trim(),
    token: $("f-token").value.trim(),
    xai_api_key: $("f-xai").value.trim(),
    model: $("f-model").value.trim(),
    persona: $("f-persona").value,
    ai_enabled: $("f-ai").checked,
    history_size: parseInt($("f-history").value, 10) || 10,
  };
  if (!input.name || !input.token) { alert("Name and token required"); return; }
  const saved = await invoke("save_bot", { input });
  state.selected = saved.id;
  await refreshBots();
  selectBot(saved.id);
}

async function deleteBot() {
  if (!state.selected) return;
  if (!confirm("Delete this bot?")) return;
  await invoke("delete_bot", { id: state.selected });
  state.selected = null;
  await refreshBots();
  showView("empty");
}

async function openDashboard(b) {
  $("dash-name").textContent = b.name;
  $("dash-status").className = "status " + (b.running ? "online" : "offline");
  $("dash-status").textContent = b.running ? "online" : "offline";
  $("guild-list").innerHTML = "";
  $("member-list").innerHTML = "";
  $("console-log").textContent = (state.logs[b.id] || []).join("\n");
  showView("dashboard");
  if (b.running) await loadGuilds();
}

async function loadGuilds() {
  try {
    const guilds = await invoke("list_guilds", { id: state.selected });
    const ul = $("guild-list");
    ul.innerHTML = "";
    for (const g of guilds) {
      const li = document.createElement("li");
      li.textContent = `${g.name} (${g.member_count})`;
      li.onclick = () => selectGuild(g);
      ul.appendChild(li);
    }
  } catch (e) { appendLog(state.selected, "error", String(e)); }
}

async function selectGuild(g) {
  state.currentGuild = g.id;
  $("members-title").textContent = `Members — ${g.name}`;
  for (const li of $("guild-list").children) li.classList.remove("active");
  event.currentTarget.classList.add("active");
  await refreshMembers();
  state.guildRoles = await invoke("list_guild_roles", { id: state.selected, guildId: g.id });
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

// Event wiring
$("new-bot-btn").onclick = () => { state.selected = null; renderBotList(); openEditor(null); };
$("save-btn").onclick = saveBot;
$("delete-btn").onclick = deleteBot;
$("start-btn").onclick = async () => {
  try { await invoke("start_bot_cmd", { id: state.selected }); await refreshBots(); const b = state.bots.find(x => x.id === state.selected); openDashboard(b); } catch (e) { alert(e); }
};
$("stop-btn").onclick = async () => {
  try { await invoke("stop_bot_cmd", { id: state.selected }); await refreshBots(); const b = state.bots.find(x => x.id === state.selected); openDashboard(b); } catch (e) { alert(e); }
};
$("edit-btn").onclick = () => { const b = state.bots.find(x => x.id === state.selected); openEditor(b); };
$("refresh-members").onclick = refreshMembers;
$("member-search").oninput = refreshMembers;
$("mm-close").onclick = () => $("member-modal").classList.add("hidden");
document.querySelectorAll("#member-modal [data-act]").forEach((b) => b.onclick = () => memberAction(b.dataset.act));

listen("bot-log", (ev) => {
  const { bot_id, level, msg } = ev.payload;
  appendLog(bot_id, level, msg);
});
listen("bot-status", async (ev) => {
  await refreshBots();
  if (state.selected === ev.payload.bot_id && state.view === "dashboard") {
    const b = state.bots.find(x => x.id === ev.payload.bot_id);
    if (b) openDashboard(b);
  }
});

refreshBots();
