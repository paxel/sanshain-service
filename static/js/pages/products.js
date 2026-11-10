// Products (#76, ADR-0011): the list of every Product and one Product's page —
// its members with their roles and origin, the Chart.yaml import and its
// Product Maintainers. Who may change what is decided on the server; the
// controls are greyed out (never hidden) for those who may not.

let productUser = null;
let productDetail = null;
let productSystems = [];
let productMemberPicker = null;
let productMaintainerPicker = null;
let productKnownUsers = [];
let productKnownGroups = [];

function productNameParam() {
  return new URLSearchParams(window.location.search).get("name");
}

function showProductView(which) {
  document.getElementById("products-list-view").classList.toggle("hidden", which !== "list");
  document.getElementById("product-view").classList.toggle("hidden", which !== "product");
}

// --- Every Product ---

async function showProductList() {
  if (productNameParam()) history.pushState({}, "", window.location.pathname);
  showProductView("list");
  const list = document.getElementById("products-list");
  try {
    const data = await fetchJSON("/admin/products");
    renderProductList(data.products || []);
  } catch (err) {
    list.innerHTML = `<div class="text-red-500 text-center py-10">Error: ${escapeHtml(err.message)}</div>`;
  }
  hideLoader();
}

function renderProductList(products) {
  const list = document.getElementById("products-list");
  if (products.length === 0) {
    list.innerHTML = `<div class="bg-white p-10 rounded-2xl border-2 border-dashed border-slate-200 text-center text-slate-500">No Products yet.</div>`;
    return;
  }
  list.innerHTML = "";
  for (const product of products) {
    const card = document.createElement("a");
    card.href = `?name=${encodeURIComponent(product.name)}`;
    card.dataset.product = product.name;
    card.className =
      "block bg-white p-4 rounded-xl border border-slate-200 shadow-xs hover:border-indigo-300";
    card.innerHTML = `<span class="font-bold text-slate-800" data-product-name></span>
      <span class="ml-3 text-xs text-slate-500" data-member-count></span>`;
    card.querySelector("[data-product-name]").textContent = product.name;
    card.querySelector("[data-member-count]").textContent =
      `${product.member_count} ${product.member_count === 1 ? "member" : "members"}`;
    card.addEventListener("click", (event) => {
      event.preventDefault();
      history.pushState({ name: product.name }, "", card.href);
      void showProduct(product.name);
    });
    list.appendChild(card);
  }
}

async function createProduct() {
  const input = document.getElementById("product-create-name");
  const name = input.value.trim();
  if (!name) return;
  const res = await apiCall("/admin/products", { method: "POST", body: { name } });
  if (!res.ok) {
    alert("Could not create the Product: " + (await errorMessage(res)));
    return;
  }
  input.value = "";
  history.pushState({ name }, "", `?name=${encodeURIComponent(name)}`);
  await showProduct(name);
}

// --- One Product ---

async function showProduct(name) {
  showProductView("product");
  document.getElementById("product-import-report").innerHTML = "";
  try {
    const res = await apiCall(`/admin/products/${encodeURIComponent(name)}`);
    if (!res.ok) {
      document.getElementById("product-name").textContent = name;
      document.getElementById("product-members").innerHTML =
        `<p class="text-red-500 text-sm">${escapeHtml(await errorMessage(res))}</p>`;
      hideLoader();
      return;
    }
    productDetail = await res.json();
  } catch (err) {
    hideLoader();
    return;
  }
  document.getElementById("product-name").textContent = productDetail.name;
  document.getElementById("product-graph-link").href =
    `/graph.html?focus=${encodeURIComponent(`product:${productDetail.name}`)}`;
  // Membership controls admit this Product's maintainers too.
  document.getElementById("product-chart-import").dataset.needsProduct = productDetail.name;
  await loadProductSystems();
  mountProductPickers();
  renderProductMembers();
  renderProductMaintainers();
  applyGates(document.getElementById("product-view"));
  hideLoader();
}

// Every system that can be added by hand: the Producers and the Consumers.
async function loadProductSystems() {
  try {
    const [producers, consumers] = await Promise.all([
      fetchJSON("/admin/producers"),
      fetchJSON("/admin/consumers"),
    ]);
    const names = new Set((producers || []).map((p) => p.name).concat(consumers || []));
    productSystems = [...names].sort(byCodeUnit);
  } catch (_) {
    productSystems = [];
  }
}

function isProductMember(system) {
  return Boolean(productDetail && productDetail.members.some((m) => m.system === system));
}

function mountProductPickers() {
  if (!productMemberPicker) {
    productMemberPicker = createPicker(document.getElementById("product-member-picker"), {
      id: "product-member-search",
      placeholder: "Tick systems to add them…",
      multiple: true,
      options: () => productSystems.map((s) => ({ value: s, label: s })),
      isSelected: (value) => isProductMember(value),
      onToggle: (value, checked) =>
        checked ? addProductMember(value) : removeProductMember(value),
    });
  }
  const memberInput = document.getElementById("product-member-search");
  memberInput.dataset.needs = "manage_producers";
  memberInput.dataset.needsProduct = productDetail.name;
  productMemberPicker.refresh();

  if (!productMaintainerPicker) {
    productMaintainerPicker = createPicker(document.getElementById("product-maintainer-picker"), {
      id: "product-maintainer-search",
      placeholder: "Tick users and groups…",
      multiple: true,
      options: () =>
        productKnownUsers
          .map((u) => ({ value: `user:${u.id}`, label: personLabel(u), group: "Users" }))
          .concat(
            productKnownGroups.map((g) => ({
              value: `group:${g.id}`,
              label: g.source === "ldap" ? `${g.name} (directory)` : g.name,
              group: "Groups",
            })),
          ),
      isSelected: (value) => isProductMaintainer(value),
      onToggle: (value, checked) => setProductMaintainer(value, checked),
    });
  }
  document.getElementById("product-maintainer-search").dataset.needs = "manage_roles";
  void loadProductMaintainerCandidates();
}

// Only a caller who may assign maintainers needs the candidates.
async function loadProductMaintainerCandidates() {
  if (!productUser || !(productUser.is_root || hasPermission(productUser, "manage_roles"))) return;
  try {
    const [users, groups] = await Promise.all([
      apiCall("/admin/users").then((r) => (r.ok ? r.json() : [])),
      apiCall("/admin/groups").then((r) => (r.ok ? r.json() : { groups: [] })),
    ]);
    productKnownUsers = users || [];
    productKnownGroups = groups.groups || [];
  } catch (_) {
    productKnownUsers = [];
    productKnownGroups = [];
  }
  productMaintainerPicker.refresh();
}

function isProductMaintainer(value) {
  const [kind, id] = value.split(":");
  const list = kind === "user" ? productDetail.maintainers.users : productDetail.maintainers.groups;
  return list.some((m) => m.id === Number(id));
}

function memberRoleChips(member) {
  if (!member.producer && !member.consumer) {
    return '<span class="text-[10px] bg-slate-200 text-slate-600 px-2 py-0.5 rounded-full font-bold uppercase" data-role="absent" title="No Producer and no Consumer of this name right now; it is live again when one provides or requires">not in Sanshain</span>';
  }
  const chips = [];
  if (member.producer) {
    chips.push(
      `<a href="/producers.html?service=${encodeURIComponent(member.system)}" data-role="producer" class="text-[10px] bg-indigo-100 text-indigo-700 px-2 py-0.5 rounded-full font-bold uppercase hover:bg-indigo-200">Producer</a>`,
    );
  }
  if (member.consumer) {
    chips.push(
      `<a href="/consumers.html?name=${encodeURIComponent(member.system)}" data-role="consumer" class="text-[10px] bg-teal-100 text-teal-700 px-2 py-0.5 rounded-full font-bold uppercase hover:bg-teal-200">Consumer</a>`,
    );
  }
  return chips.join(" ");
}

function renderProductMembers() {
  const box = document.getElementById("product-members");
  const members = productDetail.members || [];
  if (members.length === 0) {
    box.innerHTML = '<p class="text-sm text-slate-500 italic">No members yet.</p>';
    return;
  }
  const rows = members.map(
    (m) => `
      <tr class="border-t border-slate-100" data-member="${escapeHtml(m.system)}">
        <td class="py-2 pr-3 font-medium text-slate-800">${escapeHtml(m.system)}</td>
        <td class="py-2 pr-3 space-x-1">${memberRoleChips(m)}</td>
        <td class="py-2 pr-3"><span data-origin="${escapeHtml(m.origin)}" class="text-[10px] px-2 py-0.5 rounded-full font-bold uppercase ${m.origin === "helm" ? "bg-sky-100 text-sky-700" : "bg-amber-100 text-amber-700"}" title="${m.origin === "helm" ? "From the last Chart.yaml import: leaves when a chart no longer lists it" : "Added by hand: imports leave it alone"}">${m.origin === "helm" ? "Helm" : "manual"}</span></td>
        <td class="py-2 text-right"><button type="button" data-click="removeProductMember" data-click-args="${attrJson([m.system])}" data-needs="manage_producers" data-needs-product="${escapeHtml(productDetail.name)}" class="text-xs text-slate-400 hover:text-red-600">Remove</button></td>
      </tr>`,
  );
  box.innerHTML = `<table class="w-full text-sm"><thead><tr class="text-left text-xs text-slate-500"><th class="pb-1">System</th><th class="pb-1">Roles</th><th class="pb-1">Origin</th><th></th></tr></thead><tbody>${rows.join("")}</tbody></table>`;
}

function renderProductMaintainers() {
  const box = document.getElementById("product-maintainers");
  const { users, groups } = productDetail.maintainers;
  const entry = (kind, id, label) =>
    `<div class="flex items-center justify-between" data-maintainer="${kind}:${id}"><span>${escapeHtml(label)}</span><button type="button" data-click="setProductMaintainer" data-click-args="${attrJson([`${kind}:${id}`, false])}" data-needs="manage_roles" class="text-xs text-slate-400 hover:text-red-600">✕</button></div>`;
  const rows = users
    .map((u) => entry("user", u.id, personLabel(u)))
    .concat(
      groups.map((g) =>
        entry("group", g.id, g.source === "ldap" ? `${g.name} (directory)` : g.name),
      ),
    );
  box.innerHTML = rows.length
    ? rows.join("")
    : '<p class="text-sm text-slate-500 italic">Nobody maintains this Product yet.</p>';
}

async function reloadProduct() {
  if (productDetail) await showProduct(productDetail.name);
}

async function addProductMember(system) {
  const name = productDetail.name;
  const res = await apiCall(`/admin/products/${encodeURIComponent(name)}/members`, {
    method: "POST",
    body: { system },
  });
  if (!res.ok) alert("Could not add the member: " + (await errorMessage(res)));
  await reloadProduct();
}

async function removeProductMember(system) {
  const name = productDetail.name;
  const res = await apiCall(
    `/admin/products/${encodeURIComponent(name)}/members/${encodeURIComponent(system)}`,
    { method: "DELETE" },
  );
  if (!res.ok) alert("Could not remove the member: " + (await errorMessage(res)));
  await reloadProduct();
}

async function importProductChart() {
  const name = productDetail.name;
  const chartYaml = document.getElementById("product-chart-yaml").value;
  const res = await apiCall(`/admin/products/${encodeURIComponent(name)}/chart`, {
    method: "POST",
    body: { chart_yaml: chartYaml },
  });
  if (!res.ok) {
    const message = await errorMessage(res);
    document.getElementById("product-import-report").innerHTML =
      `<p class="text-red-600" data-import-error>${escapeHtml(message)}</p>`;
    return;
  }
  const report = await res.json();
  await reloadProduct();
  const line = (label, names, key) =>
    `<div data-import="${key}"><span class="font-semibold">${label}:</span> ${names.length ? names.map(escapeHtml).join(", ") : "none"}</div>`;
  document.getElementById("product-import-report").innerHTML =
    line("Added", report.added, "added") +
    line("Removed", report.removed, "removed") +
    line("Skipped (no Producer or Consumer of that name)", report.skipped, "skipped");
}

async function setProductMaintainer(subject, assign) {
  const name = encodeURIComponent(productDetail.name);
  const [kind, id] = subject.split(":");
  const res = assign
    ? await apiCall(`/admin/products/${name}/maintainers`, {
        method: "POST",
        body: kind === "user" ? { user_id: Number(id) } : { group_id: Number(id) },
      })
    : await apiCall(`/admin/products/${name}/maintainers/${kind}s/${id}`, { method: "DELETE" });
  if (!res.ok) alert(`Failed to ${assign ? "assign" : "unassign"}: ${await errorMessage(res)}`);
  await reloadProduct();
}

// A plain confirm, as for clearing system information: this page has no
// confirm modal, and the name is caller-chosen text.
async function deleteProduct() {
  const name = productDetail.name;
  if (!confirm(`Delete the Product '${name}'? Its member systems stay exactly as they are.`)) {
    return;
  }
  const res = await apiCall(`/admin/products/${encodeURIComponent(name)}`, { method: "DELETE" });
  if (!res.ok) {
    alert("Could not delete the Product: " + (await errorMessage(res)));
    return;
  }
  await showProductList();
}

async function handleProductLink() {
  const name = productNameParam();
  if (name) await showProduct(name);
  else await showProductList();
}

void checkDiscoveryAuth(async (user) => {
  productUser = user;
  await handleProductLink();
});

window.addEventListener("popstate", () => {
  void handleProductLink();
});
