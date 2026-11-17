const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const BUTTONS = [
  ["Plus", "Plus (right shift)"],
  ["Minus", "Minus (left shift)"],
  ["A", "A"],
  ["B", "B"],
  ["Y", "Y"],
  ["Z", "Z"],
  ["Up", "Up"],
  ["Down", "Down"],
  ["Left", "Left"],
  ["Right", "Right"],
  ["Tap", "Tap (custom trigger)"],
  ["DoubleTap", "Double tap (custom trigger)"],
];

const ACTIONS = [
  ["None", "Nothing"],
  ["ShiftUp", "Shift up"],
  ["ShiftDown", "Shift down"],
];

const LOG_LIMIT = 300;

const OUTPUT_FAILURE = /^(OpenBikeControl|Keyboard) output stopped/;

const KEY_NAME = /^([a-z0-9]|up|down|left|right|pageup|pagedown|home|end|space|enter|tab|minus|plus|comma|period|f([1-9]|1[0-2]))$/i;

const state = {
  saved: null,
  draft: null,
  devices: new Map(),
  clients: [],
  output: "",
  outputError: "",
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
  element("save-bar").hidden = !isDirty();
}

function appendLog(line) {
  const list = element("log");
  const item = document.createElement("li");
  const time = new Date().toLocaleTimeString();
  item.textContent = `${time}  ${line}`;
  list.prepend(item);
  while (list.children.length > LOG_LIMIT) {
    list.lastElementChild.remove();
  }
}

function renderStatus() {
  const pill = element("output-status");
  pill.classList.remove("ok", "warn");
  if (state.clients.length > 0) {
    pill.textContent = `Trainer app connected (${state.clients.length})`;
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
}

function renderMapping() {
  const body = element("mapping");
  body.replaceChildren();
  for (const [button, label] of BUTTONS) {
    const row = document.createElement("tr");
    const nameCell = document.createElement("td");
    const wrapper = document.createElement("span");
    wrapper.className = "button-name";
    const dot = document.createElement("span");
    dot.className = "dot";
    dot.id = `dot-${button}`;
    dot.setAttribute("aria-hidden", "true");
    const text = document.createElement("span");
    text.textContent = label;
    wrapper.append(dot, text);
    nameCell.append(wrapper);
    const actionCell = document.createElement("td");
    const select = document.createElement("select");
    select.setAttribute("aria-label", `Action for ${label}`);
    for (const [value, actionLabel] of ACTIONS) {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = actionLabel;
      select.append(option);
    }
    select.value = state.draft.mapping[button] || "None";
    select.addEventListener("change", () => {
      if (select.value === "None") {
        delete state.draft.mapping[button];
      } else {
        state.draft.mapping[button] = select.value;
      }
      updateSaveBar();
    });
    actionCell.append(select);
    row.append(nameCell, actionCell);
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
  element("shift-up-key").value = config.shiftUpKey;
  element("shift-down-key").value = config.shiftDownKey;
  element("focus-window").value = config.focusWindowPrefix;
  element("key-hold").value = config.keyHoldMs;
  element("double-tap").value = config.doubleTapMs;
  element("zwift-enabled").checked = config.zwiftEnabled;
  renderOutputFields();
  renderMapping();
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

function parseKey(text) {
  const value = text.trim();

  return KEY_NAME.test(value) ? value : null;
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
  const id = action === "ShiftUp" ? "shift-up" : "shift-down";
  const arrow = element(id);
  arrow.classList.add("active");
  setTimeout(() => arrow.classList.remove("active"), 220);
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
    case "button": {
      const dot = element(`dot-${event.button}`);
      if (dot) {
        dot.classList.toggle("down", event.pressed);
      }
      if (event.pressed) {
        element("last-button").textContent = `Last button: ${event.button}`;
      }
      break;
    }
    case "action":
      if (event.pressed && event.action in state.counts) {
        state.counts[event.action] += 1;
        element("count-up").textContent = state.counts.ShiftUp;
        element("count-down").textContent = state.counts.ShiftDown;
        flashArrow(event.action);
      }
      break;
    case "outputClient":
      if (event.connected) {
        state.clients.push(event.peer);
        appendLog(`Trainer app connected from ${event.peer}`);
      } else {
        state.clients = state.clients.filter((peer) => peer !== event.peer);
      }
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

async function refresh() {
  try {
    const snapshot = await invoke("get_snapshot");
    state.saved = snapshot.config;
    state.draft = clone(snapshot.config);
    state.devices = new Map(snapshot.devices.map((device) => [device.id, device]));
    state.clients = snapshot.clients;
    state.output = snapshot.output;
    state.outputError = "";
    element("autostart").checked = snapshot.autostart;
    element("config-path").textContent = snapshot.configPath;
    const list = element("log");
    list.replaceChildren();
    for (const line of snapshot.log) {
      appendLog(line);
    }
    renderForm();
    renderDevices();
    renderStatus();
  } catch (error) {
    appendLog(`Could not load settings: ${error}`);
  }
}

async function refreshStatus() {
  try {
    const snapshot = await invoke("get_snapshot");
    state.devices = new Map(snapshot.devices.map((device) => [device.id, device]));
    state.clients = snapshot.clients;
    state.output = snapshot.output;
    element("autostart").checked = snapshot.autostart;
    renderDevices();
    renderStatus();
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
    renderMapping();
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

function wireUp() {
  for (const radio of document.querySelectorAll('input[name="output"]')) {
    radio.addEventListener("change", () => {
      state.draft.output = radio.value;
      renderOutputFields();
      updateSaveBar();
    });
  }
  bindField("controller-name", "controllerName", (text) => (text.trim() ? text.trim() : null));
  bindField("obc-port", "obcPort", parseInteger(1024, 65535));
  bindField("shift-up-key", "shiftUpKey", parseKey);
  bindField("shift-down-key", "shiftDownKey", parseKey);
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
  element("revert").addEventListener("click", () => {
    state.draft = clone(state.saved);
    for (const input of document.querySelectorAll("input")) {
      input.setCustomValidity("");
    }
    renderForm();
  });
  element("swap").addEventListener("click", swap);
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
