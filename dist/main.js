const { invoke } = window.__TAURI__.core;

const servicesEl = document.querySelector("#services");
const sitesEl = document.querySelector("#sites");
const setupEl = document.querySelector("#setup");

function renderSetup(list) {
  setupEl.innerHTML = "";
  for (const { component, present } of list) {
    const li = document.createElement("li");
    li.textContent = `${component}: ${present ? "installed" : "missing"}`;
    li.className = present ? "running" : "crashed";
    setupEl.appendChild(li);
  }
}

function stateClass(state) {
  return state === "Running" ? "running" : state === "Crashed" ? "crashed" : "stopped";
}

function renderServices(list) {
  servicesEl.innerHTML = "";
  for (const { kind, state } of list) {
    const tr = document.createElement("tr");

    const nameTd = document.createElement("td");
    nameTd.textContent = kind;

    const stateTd = document.createElement("td");
    stateTd.textContent = state;
    stateTd.className = stateClass(state);

    const actionTd = document.createElement("td");
    const btn = document.createElement("button");
    const running = state === "Running";
    btn.textContent = running ? "Stop" : "Start";
    btn.addEventListener("click", async () => {
      const cmd = running ? "service_stop" : "service_start";
      try {
        renderServices(await invoke(cmd, { kind }));
      } catch (e) {
        alert(`${cmd} failed: ${e}`);
      }
    });
    actionTd.appendChild(btn);

    tr.append(nameTd, stateTd, actionTd);
    servicesEl.appendChild(tr);
  }
}

function renderSites(list) {
  sitesEl.innerHTML = "";
  if (list.length === 0) {
    const li = document.createElement("li");
    li.textContent = "No sites in www/";
    sitesEl.appendChild(li);
    return;
  }
  for (const site of list) {
    const li = document.createElement("li");
    const a = document.createElement("a");
    a.href = `https://${site.hostname}`;
    a.target = "_blank";
    a.textContent = `${site.name} — https://${site.hostname}`;
    li.appendChild(a);
    sitesEl.appendChild(li);
  }
}

async function refresh() {
  try {
    renderServices(await invoke("stack_status"));
    renderSites(await invoke("list_sites"));
    renderSetup(await invoke("setup_status"));
  } catch (e) {
    console.error(e);
  }
}

document.querySelector("#start-all").addEventListener("click", async () => {
  try {
    renderServices(await invoke("stack_start_all"));
  } catch (e) {
    alert(`start failed: ${e}`);
  }
});

document.querySelector("#stop-all").addEventListener("click", async () => {
  try {
    renderServices(await invoke("stack_stop_all"));
  } catch (e) {
    alert(`stop failed: ${e}`);
  }
});

document.querySelector("#run-setup").addEventListener("click", async () => {
  const btn = document.querySelector("#run-setup");
  btn.disabled = true;
  btn.textContent = "Installing… (authorize when prompted)";
  try {
    const report = await invoke("run_setup_cmd");
    const errs = report.errors.length ? `\nErrors:\n${report.errors.join("\n")}` : "";
    alert(`Setup done. apt: ${report.apt_packages.join(", ") || "none"}; mkcert CA: ${report.mkcert_ca}; nginx setcap: ${report.nginx_setcap}${errs}`);
    await refresh();
  } catch (e) {
    alert(`setup failed: ${e}`);
  } finally {
    btn.disabled = false;
    btn.textContent = "Install missing";
  }
});

refresh();
setInterval(refresh, 2000);
