const { invoke } = window.__TAURI__.core;

const servicesEl = document.querySelector("#services");
const sitesEl = document.querySelector("#sites");

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

refresh();
setInterval(refresh, 2000);
