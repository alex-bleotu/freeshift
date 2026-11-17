const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const PODS = [
  {
    id: "left",
    title: "Left pod",
    buttons: [
      { button: "Up", glyph: "▲", name: "Up", x: 50, y: 19 },
      { button: "Left", glyph: "◀", name: "Left", x: 25, y: 40 },
      { button: "Right", glyph: "▶", name: "Right", x: 75, y: 40 },
      { button: "Down", glyph: "▼", name: "Down", x: 50, y: 61 },
      { button: "Minus", glyph: "−", name: "Minus (shift)", shape: "paddle", x: 50, y: 86 },
    ],
  },
  {
    id: "right",
    title: "Right pod",
    buttons: [
      { button: "Y", glyph: "Y", name: "Y", x: 50, y: 19 },
      { button: "Z", glyph: "Z", name: "Z", x: 25, y: 40 },
      { button: "A", glyph: "A", name: "A", x: 75, y: 40 },
      { button: "B", glyph: "B", name: "B", x: 50, y: 61 },
      { button: "Plus", glyph: "+", name: "Plus (shift)", shape: "paddle", x: 50, y: 86 },
    ],
  },
  {
    id: "extra",
    title: "Custom trigger",
    compact: true,
    buttons: [
      { button: "Tap", glyph: "1×", name: "Tap", x: 30, y: 50 },
      { button: "DoubleTap", glyph: "2×", name: "Double tap", x: 70, y: 50 },
    ],
  },
];

const ACTION_GROUPS = [
  ["Gears", ["ShiftUp", "ShiftDown"]],
  ["Menus", ["Up", "Down", "Left", "Right", "Select", "Back", "Menu", "Home"]],
  ["Riding", ["SteerLeft", "SteerRight", "UTurn", "PowerUp", "Emote"]],
  ["View", ["CameraView", "HudToggle", "MapToggle"]],
  ["Workout", ["IncreaseDifficulty", "DecreaseDifficulty", "SkipInterval", "Pause", "Resume"]],
];

const LOG_LIMIT = 300;

const OUTPUT_FAILURE = /^(OpenBikeControl|Keyboard) output stopped/;

const KEY_NAME = /^([a-z0-9]|up|down|left|right|pageup|pagedown|home|end|space|enter|tab|escape|esc|backspace|minus|plus|comma|period|f([1-9]|1[0-2]))$/i;

const state = {
  saved: null,
  draft: null,
  devices: new Map(),
  clients: [],
  trainerApps: new Map(),
  actionLabels: new Map(),
  output: "",
  outputError: "",
  selected: null,
  lastButton: "",
  counts: { ShiftUp: 0, ShiftDown: 0 },
};

const element = (id) => document.getElementById(id);

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function isDirty() {
  return JSON.stringify(state.saved) !== JSON.stringify(state.draft);
}

function updateSaveBar() {
  const isShown = isDirty();
  element("save-bar").hidden = !isShown;
  document.body.classList.toggle("has-save-bar", isShown);
}

function actionLabel(action) {
  if (!action || action === "None") {
    return "Nothing";
  }

  return state.actionLabels.get(action) || action;
}

function buttonInfo(button) {
  for (const pod of PODS) {
    const match = pod.buttons.find((entry) => entry.button === button);
    if (match) {
      return { ...match, pod };
    }
  }

  return { button, name: button, glyph: button, pod: null };
}

function supportedActions() {
  if (!state.saved || state.saved.output !== "OpenBikeControl" || state.trainerApps.size === 0) {
    return null;
  }
  const supported = new Set();
  for (const app of state.trainerApps.values()) {
    for (const action of app.supported) {
      supported.add(action);
    }
  }

  return supported;
}

function appendLog(line) {
  const list = element("log");
  const item = document.createElement("li");
  item.textContent = `${new Date().toLocaleTimeString()}  ${line}`;
  list.prepend(item);
  while (list.children.length > LOG_LIMIT) {
    list.lastElementChild.remove();
  }
}

function renderStatus() {
  const pill = element("output-status");
  pill.classList.remove("ok", "warn");
  if (state.clients.length > 0) {
    const app = [...state.trainerApps.values()][0];
    pill.textContent = app ? `${app.app} connected` : `Trainer app connected (${state.clients.length})`;
    pill.classList.add("ok");
    return;
  }
  if (state.saved && state.saved.output === "Keyboard" && state.output) {
    pill.textContent = "Keyboard output ready";
    pill.classList.add("ok");
    return;
  }
  if (state.outputError) {
    pill.textContent = "Output stopped, see Activity";
    pill.classList.add("warn");
    return;
  }
  if (state.output) {
    pill.textContent = "Waiting for MyWhoosh to connect";
    return;
  }
  pill.textContent = "Starting";
}

function renderDevices() {
  const list = element("devices");
  list.replaceChildren();
  for (const device of state.devices.values()) {
    const item = document.createElement("li");
    const text = document.createElement("div");
    const name = document.createElement("div");
    name.className = "device-name";
    name.textContent = device.name;
    const kind = document.createElement("div");
    kind.className = "device-kind";
    kind.textContent = device.kind;
    text.append(name, kind);
    const battery = document.createElement("span");
    battery.className = "battery";
    battery.textContent = device.battery === null || device.battery === undefined ? "" : `${device.battery}% battery`;
    item.append(text, battery);
    list.append(item);
  }
  element("devices-empty").hidden = state.devices.size > 0;
  const apps = element("apps");
  apps.replaceChildren();
  for (const app of state.trainerApps.values()) {
    const item = document.createElement("li");
    const text = document.createElement("div");
    const name = document.createElement("div");
    name.className = "device-name";
    name.textContent = `${app.app} ${app.version}`.trim();
    const detail = document.createElement("div");
    detail.className = "device-kind";
    detail.textContent = `Trainer app at ${app.peer}, accepts ${app.supported.length} actions`;
    text.append(name, detail);
    item.append(text);
    apps.append(item);
  }
}

function createPadButton(entry) {
  const action = state.draft.mapping[entry.button] || "None";
  const button = document.createElement("button");
  button.type = "button";
  button.className = entry.shape === "paddle" ? "pad-button paddle" : "pad-button";
  button.dataset.button = entry.button;
  button.style.left = `${entry.x}%`;
  button.style.top = `${entry.y}%`;
  button.textContent = entry.glyph;
  button.setAttribute("aria-label", `${entry.name}: ${actionLabel(action)}. Change action`);
  button.setAttribute("aria-pressed", String(state.selected === entry.button));
  button.classList.toggle("unassigned", action === "None");
  button.addEventListener("click", () => selectButton(entry.button));

  return button;
}

function createLegendRow(entry) {
  const action = state.draft.mapping[entry.button] || "None";
  const item = document.createElement("li");
  const row = document.createElement("button");
  row.type = "button";
  row.className = "legend-row";
  row.dataset.button = entry.button;
  row.setAttribute("aria-pressed", String(state.selected === entry.button));
  const glyph = document.createElement("span");
  glyph.className = "glyph";
  glyph.setAttribute("aria-hidden", "true");
  glyph.textContent = entry.glyph;
  const name = document.createElement("span");
  name.className = "legend-name";
  name.textContent = entry.name;
  const assigned = document.createElement("span");
  assigned.className = "legend-action";
  assigned.classList.toggle("unassigned", action === "None");
  assigned.textContent = actionLabel(action);
  row.append(glyph, name, assigned);
  row.addEventListener("click", () => selectButton(entry.button));
  item.append(row);

  return item;
}

function renderPods() {
  const container = element("pods");
  container.replaceChildren();
  for (const pod of PODS) {
    const wrap = document.createElement("section");
    wrap.className = pod.compact ? "pod-wrap compact" : "pod-wrap";
    wrap.setAttribute("aria-label", pod.title);
    const title = document.createElement("h3");
    title.className = "pod-title";
    title.textContent = pod.title;
    const shape = document.createElement("div");
    shape.className = pod.compact ? "pod pod-compact" : "pod";
    for (const entry of pod.buttons) {
      shape.append(createPadButton(entry));
    }
    const legend = document.createElement("ul");
    legend.className = "legend";
    for (const entry of pod.buttons) {
      legend.append(createLegendRow(entry));
    }
    wrap.append(title, shape, legend);
    container.append(wrap);
  }
}

function renderEditor() {
  const editor = element("editor");
  if (!state.selected || !editor.open) {
    return;
  }
  const info = buttonInfo(state.selected);
  const current = state.draft.mapping[state.selected] || "None";
  element("editor-title").textContent = info.pod ? `${info.name} on the ${info.pod.title.toLowerCase()}` : info.name;
  element("editor-current").textContent = `Now: ${actionLabel(current)}`;
  const supported = supportedActions();
  const isKeyboard = state.draft.output === "Keyboard";
  const groups = element("editor-groups");
  groups.replaceChildren();
  const sections = [["", ["None"]], ...ACTION_GROUPS];
  for (const [heading, actions] of sections) {
    const group = document.createElement("div");
    group.className = "chip-group";
    if (heading) {
      const label = document.createElement("h4");
      label.textContent = heading;
      group.append(label);
    }
    const chips = document.createElement("div");
    chips.className = "chips";
    for (const action of actions) {
      const chip = document.createElement("button");
      chip.type = "button";
      chip.className = "chip";
      chip.textContent = actionLabel(action);
      chip.setAttribute("aria-pressed", String(current === action));
      const hasNoKey = isKeyboard && action !== "None" && !(state.draft.keys[action] || "").trim();
      const isUnsupported = !isKeyboard && supported !== null && action !== "None" && !supported.has(action);
      if (hasNoKey) {
        chip.classList.add("unsupported");
        chip.title = "No key set for this action in keyboard mode";
      } else if (isUnsupported) {
        chip.classList.add("unsupported");
        chip.title = "The connected trainer app did not list this action";
      }
      chip.addEventListener("click", () => {
        assignAction(state.selected, action);
        editor.close();
      });
      chips.append(chip);
    }
    group.append(chips);
    groups.append(group);
  }
}

function selectButton(button) {
  const editor = element("editor");
  state.selected = button;
  renderPods();
  editor.showModal();
  renderEditor();
  const current = editor.querySelector('.chip[aria-pressed="true"]');
  if (current) {
    current.focus();
  }
}

function closeEditor() {
  const button = state.selected;
  state.selected = null;
  renderPods();
  if (button) {
    const pad = document.querySelector(`.pad-button[data-button="${button}"]`);
    if (pad) {
      pad.focus();
    }
  }
}

function assignAction(button, action) {
  if (action === "None") {
    delete state.draft.mapping[button];
  } else {
    state.draft.mapping[button] = action;
  }
  renderPods();
  renderKeys();
  updateSaveBar();
}

function usedActions() {
  const used = new Set(Object.values(state.draft.mapping).filter((action) => action !== "None"));
  const ordered = ACTION_GROUPS.flatMap(([, actions]) => actions);

  return ordered.filter((action) => used.has(action));
}

function renderKeys() {
  const body = element("keys");
  body.replaceChildren();
  for (const action of usedActions()) {
    const row = document.createElement("tr");
    const labelCell = document.createElement("td");
    labelCell.textContent = actionLabel(action);
    const inputCell = document.createElement("td");
    const input = document.createElement("input");
    input.type = "text";
    input.maxLength = 10;
    input.value = state.draft.keys[action] || "";
    input.setAttribute("aria-label", `Key for ${actionLabel(action)}`);
    input.addEventListener("input", () => {
      const value = input.value.trim();
      if (value && !KEY_NAME.test(value)) {
        input.setCustomValidity("Unknown key");
        return;
      }
      input.setCustomValidity("");
      if (value) {
        state.draft.keys[action] = value;
      } else {
        delete state.draft.keys[action];
      }
      renderEditor();
      updateSaveBar();
    });
    inputCell.append(input);
    row.append(labelCell, inputCell);
    body.append(row);
  }
}

function renderTriggers() {
  const list = element("triggers");
  list.replaceChildren();
  state.draft.customTriggers.forEach((trigger, index) => {
    const item = document.createElement("li");
    const text = document.createElement("code");
    text.textContent = `${trigger.namePrefix}  ${trigger.characteristic}`;
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "link";
    remove.textContent = "Remove";
    remove.setAttribute("aria-label", `Remove trigger ${trigger.namePrefix}`);
    remove.addEventListener("click", () => {
      state.draft.customTriggers.splice(index, 1);
      renderTriggers();
      updateSaveBar();
    });
    item.append(text, remove);
    list.append(item);
  });
}

function renderOutputFields() {
  const isKeyboard = state.draft.output === "Keyboard";
  element("keyboard-fields").hidden = !isKeyboard;
  element("obc-fields").hidden = isKeyboard;
  element("obc-name-hint").textContent = state.draft.controllerName || "FreeShift";
}

function renderForm() {
  const config = state.draft;
  for (const radio of document.querySelectorAll('input[name="output"]')) {
    radio.checked = radio.value === config.output;
  }
  element("controller-name").value = config.controllerName;
  element("obc-port").value = config.obcPort;
  element("focus-window").value = config.focusWindowPrefix;
  element("key-hold").value = config.keyHoldMs;
  element("double-tap").value = config.doubleTapMs;
  element("zwift-enabled").checked = config.zwiftEnabled;
  renderOutputFields();
  renderPods();
  renderEditor();
  renderKeys();
  renderTriggers();
  updateSaveBar();
}

function bindField(id, key, parse) {
  element(id).addEventListener("input", (event) => {
    const value = parse(event.target.value);
    if (value === null) {
      event.target.setCustomValidity("Invalid value");
      return;
    }
    event.target.setCustomValidity("");
    state.draft[key] = value;
    if (key === "controllerName") {
      renderOutputFields();
    }
    updateSaveBar();
  });
}

function parseInteger(minimum, maximum) {
  return (text) => {
    const value = Number.parseInt(text, 10);
    if (Number.isNaN(value) || value < minimum || value > maximum) {
      return null;
    }

    return value;
  };
}

function flashArrow(action) {
  const arrow = element(action === "ShiftUp" ? "shift-up" : "shift-down");
  arrow.classList.add("active");
  setTimeout(() => arrow.classList.remove("active"), 220);
}

function markPressed(button, isPressed) {
  for (const node of document.querySelectorAll(`[data-button="${button}"]`)) {
    node.classList.toggle("down", isPressed);
  }
}

function handleEvent(event) {
  switch (event.type) {
    case "log":
      appendLog(event.message);
      if (OUTPUT_FAILURE.test(event.message)) {
        state.outputError = event.message;
        renderStatus();
      }
      break;
    case "deviceConnected":
      state.devices.set(event.id, { id: event.id, name: event.name, kind: event.kind, battery: null });
      appendLog(`Connected ${event.name} (${event.kind})`);
      renderDevices();
      break;
    case "deviceDisconnected":
      state.devices.delete(event.id);
      appendLog(`Disconnected ${event.name}`);
      renderDevices();
      break;
    case "battery": {
      const device = state.devices.get(event.id);
      if (device) {
        device.battery = event.level;
        renderDevices();
      }
      break;
    }
    case "unlocked":
      appendLog(`${event.id} answered the handshake`);
      break;
    case "button":
      markPressed(event.button, event.pressed);
      if (event.pressed) {
        state.lastButton = buttonInfo(event.button).name;
        const action = state.draft.mapping[event.button] || "None";
        if (action === "None") {
          element("last-action").textContent = `${state.lastButton}: nothing assigned`;
        }
      }
      break;
    case "action":
      if (event.pressed) {
        element("last-action").textContent = `${actionLabel(event.action)}${state.lastButton ? ` (${state.lastButton})` : ""}`;
        if (event.action in state.counts) {
          state.counts[event.action] += 1;
          element("count-up").textContent = state.counts.ShiftUp;
          element("count-down").textContent = state.counts.ShiftDown;
          flashArrow(event.action);
        }
      }
      break;
    case "outputClient":
      if (event.connected) {
        state.clients.push(event.peer);
        appendLog(`Trainer app connected from ${event.peer}`);
      } else {
        state.clients = state.clients.filter((peer) => peer !== event.peer);
        state.trainerApps.delete(event.peer);
        renderDevices();
        renderEditor();
      }
      renderStatus();
      break;
    case "trainerApp":
      state.trainerApps.set(event.peer, { peer: event.peer, app: event.app, version: event.version, supported: event.supported });
      appendLog(`${event.peer} is ${event.app} ${event.version}`);
      renderDevices();
      renderEditor();
      renderStatus();
      break;
    case "outputReady":
      state.output = event.description;
      appendLog(event.description);
      renderStatus();
      break;
    case "notification":
      appendLog(`${event.device} ${event.characteristic}: ${event.hex}`);
      break;
    default:
      break;
  }
}

function applyStatus(snapshot) {
  state.devices = new Map(snapshot.devices.map((device) => [device.id, device]));
  state.clients = snapshot.clients;
  state.trainerApps = new Map(snapshot.trainerApps.map((app) => [app.peer, app]));
  state.output = snapshot.output;
  element("autostart").checked = snapshot.autostart;
  renderDevices();
  renderStatus();
}

async function refresh() {
  try {
    const snapshot = await invoke("get_snapshot");
    state.actionLabels = new Map(snapshot.actions.map((action) => [action.id, action.label]));
    state.saved = snapshot.config;
    state.draft = clone(snapshot.config);
    state.outputError = "";
    element("config-path").textContent = snapshot.configPath;
    const list = element("log");
    list.replaceChildren();
    for (const line of snapshot.log) {
      appendLog(line);
    }
    renderForm();
    applyStatus(snapshot);
  } catch (error) {
    appendLog(`Could not load settings: ${error}`);
  }
}

async function refreshStatus() {
  try {
    applyStatus(await invoke("get_snapshot"));
  } catch (error) {
    appendLog(`Could not read status: ${error}`);
  }
}

async function save() {
  const button = element("save");
  button.disabled = true;
  try {
    await invoke("save_config", { config: state.draft });
    state.saved = clone(state.draft);
    updateSaveBar();
    renderEditor();
  } catch (error) {
    appendLog(`Save failed: ${error}`);
  } finally {
    button.disabled = false;
  }
}

async function swap() {
  if (isDirty()) {
    for (const button of Object.keys(state.draft.mapping)) {
      const action = state.draft.mapping[button];
      if (action === "ShiftUp") {
        state.draft.mapping[button] = "ShiftDown";
      } else if (action === "ShiftDown") {
        state.draft.mapping[button] = "ShiftUp";
      }
    }
    renderPods();
    renderEditor();
    updateSaveBar();
    return;
  }
  try {
    const config = await invoke("swap_directions");
    state.saved = config;
    state.draft = clone(config);
    renderForm();
    appendLog("Swapped shift up and shift down");
  } catch (error) {
    appendLog(`Swap failed: ${error}`);
  }
}

async function toggleAutostart(event) {
  const checkbox = event.target;
  try {
    checkbox.checked = await invoke("set_autostart", { enabled: checkbox.checked });
  } catch (error) {
    checkbox.checked = !checkbox.checked;
    appendLog(`Start with Windows failed: ${error}`);
  }
}

function addTrigger(event) {
  event.preventDefault();
  const form = event.target;
  if (!form.reportValidity()) {
    return;
  }
  state.draft.customTriggers.push({
    namePrefix: element("trigger-name").value.trim(),
    characteristic: element("trigger-uuid").value.trim().toLowerCase(),
  });
  form.reset();
  renderTriggers();
  updateSaveBar();
}

function revert() {
  state.draft = clone(state.saved);
  for (const input of document.querySelectorAll("input")) {
    input.setCustomValidity("");
  }
  renderForm();
}

function wireUp() {
  for (const radio of document.querySelectorAll('input[name="output"]')) {
    radio.addEventListener("change", () => {
      state.draft.output = radio.value;
      renderOutputFields();
      renderEditor();
      updateSaveBar();
    });
  }
  bindField("controller-name", "controllerName", (text) => (text.trim() ? text.trim() : null));
  bindField("obc-port", "obcPort", parseInteger(1024, 65535));
  bindField("focus-window", "focusWindowPrefix", (text) => text);
  bindField("key-hold", "keyHoldMs", parseInteger(10, 500));
  bindField("double-tap", "doubleTapMs", parseInteger(150, 800));
  element("zwift-enabled").addEventListener("change", (event) => {
    state.draft.zwiftEnabled = event.target.checked;
    updateSaveBar();
  });
  element("autostart").addEventListener("change", toggleAutostart);
  element("trigger-form").addEventListener("submit", addTrigger);
  element("save").addEventListener("click", save);
  element("revert").addEventListener("click", revert);
  element("swap").addEventListener("click", swap);
  const settings = element("settings");
  element("open-settings").addEventListener("click", () => settings.showModal());
  element("settings-close").addEventListener("click", () => settings.close());
  settings.addEventListener("click", (event) => {
    if (event.target === settings) {
      settings.close();
    }
  });
  settings.addEventListener("close", () => element("open-settings").focus());
  const editor = element("editor");
  element("editor-close").addEventListener("click", () => editor.close());
  editor.addEventListener("close", closeEditor);
  editor.addEventListener("click", (event) => {
    if (event.target === editor) {
      editor.close();
    }
  });
}

async function showVersion() {
  try {
    element("version").textContent = await window.__TAURI__.app.getVersion();
  } catch (error) {
    appendLog(`Could not read the app version: ${error}`);
  }
}

async function start() {
  wireUp();
  await listen("engine-event", (message) => handleEvent(message.payload));
  await listen("engine-restarted", () => {
    state.outputError = "";
    refreshStatus();
  });
  await refresh();
  showVersion();
}

start();
