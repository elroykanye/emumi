const $ = (selector, root = document) => root.querySelector(selector);
const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
const icon = (name) => `<svg aria-hidden="true"><use href="#i-${name}"></use></svg>`;

let state = null;
let selected = null;
let activePage = "androids";
let requestInFlight = false;
let toastTimer;
const pending = new Map();

async function api(path, options = {}) {
  const response = await fetch(path, {
    headers: { "Content-Type": "application/json", ...(options.headers || {}) },
    ...options,
  });
  const body = await response.json().catch(() => ({ message: "Unexpected response" }));
  if (!response.ok) throw new Error(body.message || `Request failed (${response.status})`);
  return body;
}

async function refresh({ quiet = true } = {}) {
  if (requestInFlight) return;
  requestInFlight = true;
  try {
    state = await api("/api/state");
    for (const [name, action] of pending) {
      const profile = state.profiles.find((item) => item.name === name);
      if (!profile || (action === "starting" && profile.running_serial) || (action === "stopping" && !profile.running_serial)) pending.delete(name);
    }
    if (!selected || !state.profiles.some((p) => p.name === selected)) selected = state.profiles[0]?.name || null;
    render();
  } catch (error) {
    if (!quiet) toast(error.message, true);
  } finally {
    requestInFlight = false;
  }
}

function render() {
  renderHealth();
  renderProfiles();
  renderDetail();
  renderMonitor();
  renderLogs();
  renderSettings();
  renderCreateImages();
}

function renderHealth() {
  const ok = state.health.emulator && state.health.adb && state.health.kvm;
  const summary = $("#health-summary");
  summary.className = `health-summary ${ok ? "good" : "warn"}`;
  summary.innerHTML = `<strong>${ok ? "System ready" : "Setup needed"}</strong>${ok ? "SDK, ADB, and KVM found" : "Open Settings for details"}`;
  const entries = [
    ["Android Emulator", state.health.emulator, "Runs virtual devices"],
    ["ADB", state.health.adb, "Tracks live emulator state"],
    ["AVD Manager", state.health.avd_manager, "Creates and deletes profiles"],
    ["KVM acceleration", state.health.kvm, "Fast Linux virtualization"],
    ["Java", state.health.java, "Runs SDK management commands"],
  ];
  $("#health-list").innerHTML = entries.map(([name, ready, description]) => `<div class="health-row">${icon(ready ? "check" : "alert")}<div><strong>${name}</strong><small>${description}</small></div><span class="pill ${ready ? "ready" : "issue"}">${ready ? "Ready" : "Needs attention"}</span></div>`).join("");
}

function renderProfiles() {
  const list = $("#profile-list");
  if (!state.profiles.length) {
    list.innerHTML = empty("devices", "No Androids yet", "Create one to get started.");
    return;
  }
  list.innerHTML = state.profiles.map((profile) => {
    const running = Boolean(profile.running_serial), transition = pending.get(profile.name);
    const status = transition === "starting" ? "Starting…" : transition === "stopping" ? "Stopping…" : running ? "Running" : "Stopped";
    return `<button class="profile-card ${selected === profile.name ? "active" : ""} ${running ? "running" : ""}" data-profile="${escapeHtml(profile.name)}"><span class="device-icon">${icon("devices")}</span><span><span class="profile-name">${escapeHtml(profile.name)}</span><span class="profile-meta">${escapeHtml(profile.device_name || "Android device")}${profile.api_level ? ` · API ${profile.api_level}` : ""}</span></span><span class="pill ${running ? "running" : ""}">${status}</span></button>`;
  }).join("");
  $$('[data-profile]').forEach((button) => button.onclick = () => { selected = button.dataset.profile; renderProfiles(); renderDetail(); });
}

function renderDetail() {
  const container = $("#profile-detail");
  const profile = state.profiles.find((item) => item.name === selected);
  if (!profile) { container.innerHTML = empty("devices", "Select an Android", "Its controls will appear here."); return; }
  const running = Boolean(profile.running_serial), transition = pending.get(profile.name);
  const options = normalizeOptions(state.profile_options[profile.name]);
  container.innerHTML = `
    <div class="card"><div class="hero"><span class="hero-icon">${icon("android")}</span><div><h2>${escapeHtml(profile.name)}</h2><p>${escapeHtml(profile.device_name || "Android device")}${profile.api_level ? ` · Android API ${profile.api_level}` : ""}</p></div><div class="hero-actions"><button id="delete-profile" class="secondary danger">${icon("trash")}Delete</button><button id="run-profile" class="primary" ${transition ? "disabled" : ""}>${icon(transition ? "refresh" : running ? "stop" : "play")}${transition === "starting" ? "Starting…" : transition === "stopping" ? "Stopping…" : running ? "Stop" : "Start"}</button></div></div><div class="detail-pills"><span class="pill ${running ? "running" : ""}">${transition === "starting" ? "Starting…" : transition === "stopping" ? "Stopping…" : running ? "Running" : "Stopped"}</span>${profile.resolution ? `<span class="pill">${escapeHtml(profile.resolution)}</span>` : ""}${profile.running_serial ? `<span class="pill">${escapeHtml(profile.running_serial)}</span>` : ""}</div></div>
    <div class="card"><div class="card-head"><div><h2>Quick setup</h2><p>Useful choices without emulator jargon</p></div><button id="save-profile" class="secondary">Save</button></div>
      ${settingSegments("cpu", "Performance", "CPU and memory used when Android starts", options.speed, ["Efficient","Balanced","Fast"])}
      ${settingToggle("keyboard", "Use computer keyboard", "Send Linux keyboard input into Android", "host_keyboard", options.host_keyboard)}
      ${settingToggle("monitor", "Show device frame", "Leave off for a cleaner, easier-to-resize window", "device_frame", options.device_frame)}
      <details><summary>Advanced settings</summary><div class="advanced-grid"><label class="field"><span>CPU cores</span><small>Virtual processors</small><input id="cores" type="number" min="1" max="16" value="${options.cores}"></label><label class="field"><span>Memory</span><small>Guest RAM in MB</small><input id="memory" type="number" min="512" max="16384" step="256" value="${options.memory_mb}"></label><label class="field"><span>Graphics</span><small>Rendering backend</small><select id="gpu"><option value="auto">Automatic</option><option value="host">Hardware</option><option value="swiftshader_indirect">Software</option></select></label><label class="field"><span>ADB port</span><small>Blank means automatic</small><input id="port" type="number" min="5554" max="5682" value="${options.adb_port || ""}"></label><label class="setting-row"><span class="setting-icon">${icon("refresh")}</span><span class="setting-copy"><strong>Cold boot</strong><small>Ignore saved state next time</small></span><span class="switch"><input id="cold-boot" type="checkbox" ${options.cold_boot ? "checked" : ""}><span></span></span></label></div></details>
    </div>`;
  $("#gpu").value = options.gpu_mode;
  $("#run-profile").onclick = () => toggleRun(profile, running);
  $("#delete-profile").onclick = () => deleteProfile(profile.name, running);
  $("#save-profile").onclick = () => saveProfile(profile.name, options);
  $$('[data-speed]').forEach((button) => button.onclick = () => {
    $$('[data-speed]').forEach((item) => item.classList.remove("active"));
    button.classList.add("active");
    const presets = { Efficient:[2,2048], Balanced:[4,4096], Fast:[8,8192] };
    $("#cores").value = presets[button.dataset.speed][0]; $("#memory").value = presets[button.dataset.speed][1];
  });
}

function settingSegments(iconName, title, detail, value, choices) {
  return `<div class="setting-row"><span class="setting-icon">${icon(iconName === "cpu" ? "cpu" : iconName)}</span><span class="setting-copy"><strong>${title}</strong><small>${detail}</small></span><span class="segments">${choices.map((choice) => `<button data-speed="${choice}" class="${value === choice ? "active" : ""}">${choice}</button>`).join("")}</span></div>`;
}

function settingToggle(iconName, title, detail, id, checked) {
  return `<label class="setting-row"><span class="setting-icon">${icon(iconName)}</span><span class="setting-copy"><strong>${title}</strong><small>${detail}</small></span><span class="switch"><input id="${id}" type="checkbox" ${checked ? "checked" : ""}><span></span></span></label>`;
}

function normalizeOptions(value = {}) {
  return { speed:"Balanced", picture:"Phone", window:"Remember", cores:4, memory_mb:4096, dpi:420, adb_port:null, gpu_mode:"auto", cold_boot:false, host_keyboard:true, device_frame:false, ...value };
}

async function saveProfile(name, existing) {
  const speed = $('[data-speed].active')?.dataset.speed || existing.speed;
  const options = { ...existing, speed, cores:Number($("#cores").value), memory_mb:Number($("#memory").value), gpu_mode:$("#gpu").value, adb_port:$("#port").value ? Number($("#port").value) : null, cold_boot:$("#cold-boot").checked, host_keyboard:$("#host_keyboard").checked, device_frame:$("#device_frame").checked };
  await action(`/api/profiles/${encodeURIComponent(name)}/settings`, { method:"POST", body:JSON.stringify(options) });
}

async function toggleRun(profile, running) {
  pending.set(profile.name, running ? "stopping" : "starting");
  renderProfiles(); renderDetail();
  try {
    const result = await api(`/api/profiles/${encodeURIComponent(profile.name)}/${running ? "stop" : "start"}`, { method:"POST" });
    toast(result.message);
    await refresh({ quiet:false });
  } catch (error) {
    pending.delete(profile.name);
    renderProfiles(); renderDetail();
    toast(error.message, true);
  }
}

async function deleteProfile(name, running) {
  if (running) { toast("Stop this Android before deleting it", true); return; }
  if (!confirm(`Delete “${name}”? Its apps and data will be removed.`)) return;
  await action(`/api/profiles/${encodeURIComponent(name)}`, { method:"DELETE" });
}

function renderMonitor() {
  const total = state.stats.memory_total_bytes || 0, used = state.stats.memory_used_bytes || 0;
  const running = state.profiles.filter((p) => p.running_serial);
  $("#metrics").innerHTML = metric("cpu", "Host CPU", `${Math.round(state.stats.cpu_percent)}%`, state.stats.cpu_percent) + metric("memory", "Host memory", `${gib(used)} / ${gib(total)} GB`, total ? used / total * 100 : 0) + metric("devices", "Running", `${running.length} Android${running.length === 1 ? "" : "s"}`, state.profiles.length ? running.length / state.profiles.length * 100 : 0);
  $("#live-list").innerHTML = running.length ? running.map((p) => `<div class="live-row">${icon("check")}<div><strong>${escapeHtml(p.name)}</strong><small>${escapeHtml(p.device_name || "Android device")}</small></div><span class="pill running">${escapeHtml(p.running_serial)}</span></div>`).join("") : empty("devices", "Nothing is running", "Start an Android to see it here.");
}

function metric(iconName, title, value, percent) { return `<div class="metric"><div class="metric-head">${icon(iconName)}${title}</div><strong>${value}</strong><div class="bar"><span style="width:${Math.max(0,Math.min(100,percent))}%"></span></div></div>`; }
function gib(bytes) { return (bytes / 1024 / 1024 / 1024).toFixed(1); }

function renderLogs() {
  $("#log-list").innerHTML = state.logs.length ? [...state.logs].reverse().map((entry) => `<div class="log-row"><span class="log-dot ${entry.level}"></span><div><strong>${escapeHtml(entry.message)}</strong><small>${entry.level}</small></div><small>${relativeTime(entry.at)}</small></div>`).join("") : empty("log", "No activity yet", "Actions will appear here.");
}

function renderSettings() {
  if (document.activeElement !== $("#sdk-path")) $("#sdk-path").value = state.config.android_sdk_path || "";
  if (document.activeElement !== $("#jdk-path")) $("#jdk-path").value = state.config.jdk_path || "";
}

function renderCreateImages() {
  const select = $("#create-image");
  const current = select.value;
  select.innerHTML = state.system_images.length ? state.system_images.map((image) => `<option value="${escapeHtml(image.package_id)}">${escapeHtml(image.label)}</option>`).join("") : `<option value="">No installed system image</option>`;
  if ([...select.options].some((option) => option.value === current)) select.value = current;
}

async function action(path, options) {
  try { const result = await api(path, options); toast(result.message); await refresh({ quiet:false }); }
  catch (error) { toast(error.message, true); }
}

function toast(message, error = false) {
  const el = $("#toast"); el.textContent = message; el.className = error ? "show error" : "show";
  clearTimeout(toastTimer); toastTimer = setTimeout(() => el.className = "", 3200);
}

function empty(iconName, title, detail) { return `<div class="empty">${icon(iconName)}<strong>${title}</strong><p>${detail}</p></div>`; }
function escapeHtml(value = "") { return String(value).replace(/[&<>'"]/g, (char) => ({"&":"&amp;","<":"&lt;",">":"&gt;","'":"&#39;",'"':"&quot;"}[char])); }
function relativeTime(seconds) { const ago = Math.max(0, Math.floor(Date.now()/1000) - seconds); return ago < 5 ? "now" : ago < 60 ? `${ago}s ago` : `${Math.floor(ago/60)}m ago`; }

$$('.nav').forEach((button) => button.onclick = () => {
  activePage = button.dataset.page;
  $$('.nav').forEach((item) => item.classList.toggle("active", item === button));
  $$('.page').forEach((page) => page.classList.toggle("active", page.id === `page-${activePage}`));
});
$("#refresh").onclick = () => refresh({ quiet:false });
$("#new-profile").onclick = () => { $("#create-name").value = ""; $("#create-error").textContent = ""; $("#create-dialog").showModal(); };
$("#create-dialog").addEventListener("close", async () => {
  if ($("#create-dialog").returnValue !== "default") return;
  const request = { name:$("#create-name").value.trim(), device_id:$("#create-device").value, package_id:$("#create-image").value };
  try { const result = await api("/api/profiles", { method:"POST", body:JSON.stringify(request) }); selected = request.name; toast(result.message); await refresh({ quiet:false }); }
  catch (error) { $("#create-error").textContent = error.message; $("#create-dialog").showModal(); }
});
$("#save-settings").onclick = () => action("/api/settings", { method:"POST", body:JSON.stringify({ android_sdk_path:$("#sdk-path").value, jdk_path:$("#jdk-path").value }) });

refresh({ quiet:false });
setInterval(() => refresh(), 2000);
