// One picker for services, users and groups. Typing filters the list; there is
// no accept button. A single-value picker takes an entry the moment it is
// picked. A multi-value picker shows a checkmark per entry, and ticking or
// unticking one takes effect at once, without closing the list.
//
// createPicker(root, config) fills `root` (an empty element) and returns
// { refresh() } to redraw after the options or the selection changed.
// config:
//   id           id for the text input (tests and labels use it)
//   placeholder  text shown in the empty input
//   multiple     true for checkmarks, false for a single value
//   options()    [{ value, label, group? }], read on every redraw
//   isSelected(value)
//   onToggle(value, checked)   multiple: an entry was ticked or unticked
//   onPick(value)              single: an entry was picked
//   onFreeText(text)           optional: Enter on text that matches nothing
//   selectedLabel()            single: what the closed input shows
//   onSearch(text)             optional: called as the text changes, e.g. to
//                              look entries up on the server and refresh()

function createPicker(root, config) {
  const input = document.createElement("input");
  input.type = "text";
  input.autocomplete = "off";
  if (config.id) input.id = config.id;
  input.placeholder = config.placeholder || "";
  input.setAttribute("role", "combobox");
  input.setAttribute("aria-expanded", "false");
  input.className =
    "w-full px-3 py-1.5 rounded-lg border border-slate-300 text-xs focus:outline-hidden focus:ring-2 focus:ring-indigo-500 shadow-xs bg-white";
  const panel = document.createElement("div");
  panel.setAttribute("role", "listbox");
  panel.className =
    "hidden absolute z-30 mt-1 left-0 min-w-full w-max max-w-[24rem] max-h-72 overflow-y-auto bg-white border border-slate-200 rounded-lg shadow-lg py-1";
  root.classList.add("relative");
  root.append(input, panel);

  let highlighted = 0;
  let open = false;
  // A single-value picker shows its choice in the input; that text is no
  // search until the person types.
  let typed = false;

  const query = () => (typed ? input.value.trim().toLowerCase() : "");

  // Matches of the typed text, an exact match first so Enter takes it.
  function matches() {
    const text = query();
    const all = config.options();
    if (!text) return all;
    const hits = all.filter(
      (o) => o.label.toLowerCase().includes(text) || String(o.value).toLowerCase().includes(text),
    );
    const exact = (o) => o.label.toLowerCase() === text || String(o.value).toLowerCase() === text;
    return hits.filter(exact).concat(hits.filter((o) => !exact(o)));
  }

  function draw() {
    const list = matches();
    highlighted = Math.min(highlighted, Math.max(0, list.length - 1));
    panel.replaceChildren();
    if (list.length === 0) {
      const empty = document.createElement("div");
      empty.className = "px-3 py-1.5 text-xs text-slate-400 italic";
      empty.textContent = config.onFreeText && query() ? "Enter to use this text" : "No match";
      panel.append(empty);
      return;
    }
    let group = null;
    list.forEach((option, index) => {
      if (option.group && option.group !== group) {
        group = option.group;
        const heading = document.createElement("div");
        heading.className =
          "px-3 pt-2 pb-0.5 text-[10px] font-bold uppercase tracking-wider text-slate-400";
        heading.textContent = group;
        panel.append(heading);
      }
      const selected = config.isSelected(option.value);
      const item = document.createElement("div");
      item.setAttribute("role", "option");
      item.dataset.pickerOption = option.value;
      item.setAttribute("aria-selected", String(selected));
      if (config.multiple) item.setAttribute("aria-checked", String(selected));
      item.className = `flex items-center gap-2 px-3 py-1.5 text-xs cursor-pointer ${
        index === highlighted ? "bg-indigo-50" : "hover:bg-slate-50"
      } ${selected ? "text-indigo-700 font-medium" : "text-slate-700"}`;
      const mark = document.createElement("span");
      mark.className = "w-3 text-indigo-600";
      mark.textContent = selected ? "✓" : "";
      const label = document.createElement("span");
      label.textContent = option.label;
      item.append(mark, label);
      // mousedown, not click: the input keeps its focus, so the list stays open.
      item.addEventListener("mousedown", (event) => {
        event.preventDefault();
        choose(option);
      });
      panel.append(item);
    });
  }

  function show() {
    open = true;
    panel.classList.remove("hidden");
    input.setAttribute("aria-expanded", "true");
    draw();
  }

  function hide() {
    open = false;
    typed = false;
    panel.classList.add("hidden");
    input.setAttribute("aria-expanded", "false");
    if (!config.multiple && config.selectedLabel) input.value = config.selectedLabel();
  }

  async function choose(option) {
    if (config.multiple) {
      await config.onToggle(option.value, !config.isSelected(option.value));
      draw();
    } else {
      await config.onPick(option.value);
      hide();
      input.blur();
    }
  }

  input.addEventListener("focus", () => {
    if (!config.multiple) input.select();
    show();
  });
  input.addEventListener("blur", hide);
  input.addEventListener("input", () => {
    typed = true;
    highlighted = 0;
    if (config.onSearch) config.onSearch(input.value.trim());
    show();
  });
  input.addEventListener("keydown", (event) => {
    const list = matches();
    if (event.key === "ArrowDown") {
      event.preventDefault();
      highlighted = Math.min(highlighted + 1, list.length - 1);
      show();
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      highlighted = Math.max(highlighted - 1, 0);
      show();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const text = input.value.trim();
      // Enter takes the entry and closes the list, so it covers nothing;
      // typing again reopens it. Clicks keep a multi-value list open.
      if (list.length) {
        choose(list[highlighted]);
        if (config.multiple) {
          input.value = "";
          hide();
        }
      } else if (text && config.onFreeText) {
        config.onFreeText(text);
        input.value = "";
        hide();
      }
    } else if (event.key === "Escape") {
      input.blur();
    }
  });

  if (!config.multiple && config.selectedLabel) input.value = config.selectedLabel();
  return {
    input,
    refresh() {
      if (open) draw();
      else if (!config.multiple && config.selectedLabel) input.value = config.selectedLabel();
    },
  };
}
