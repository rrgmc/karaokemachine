// The two tabs, and the words panel: the song Select chose, the words box, and the tick that says
// to use the box.
//
// The panel's `data-` attributes say what the stylesheet has to know to hold a Start the server
// would refuse: whether a song is selected, whether its synced copy exists, and whether it needs
// words. The body carries `has-words` while the tick is on.
(() => {
  const panel = document.getElementById("words");
  const box = document.getElementById("words-box");
  const use = document.getElementById("use-words");
  if (!panel || !box || !use) return;
  const song = document.getElementById("selected-song");
  const name = document.getElementById("selected-name");
  const notes = document.getElementById("selected-notes");
  const force = document.getElementById("force");

  const mark = () => document.body.classList.toggle("has-words", use.checked);
  use.addEventListener("change", mark);
  // Words put into an empty box are words somebody means to use, so the tick goes on with them.
  // It stays theirs to take off, and words already in the box change nothing.
  let held = box.value.trim() !== "";
  box.addEventListener("input", () => {
    const holds = box.value.trim() !== "";
    if (holds && !held) use.checked = true;
    if (!holds) use.checked = false;
    held = holds;
    mark();
  });

  // Shows a row's song in the panel. `fresh` is a press of Select, which asks again before a
  // synced copy is replaced. A redraw of the rows keeps the answer already given.
  const show = (button, fresh) => {
    const row = button.closest("tr");
    document.querySelectorAll("tr.selected").forEach((other) => other.classList.remove("selected"));
    row.classList.add("selected");
    song.value = button.dataset.select;
    name.textContent = button.dataset.name;
    notes.replaceChildren(...[...row.querySelectorAll(".hint")].map((note) => note.cloneNode(true)));
    panel.toggleAttribute("data-selected", true);
    panel.toggleAttribute("data-out-exists", "outExists" in button.dataset);
    panel.toggleAttribute("data-needs-words", "needsWords" in button.dataset);
    // A press of Select starts each name as what the song states. A redraw of the rows keeps
    // what somebody has typed since.
    if (fresh) {
      for (const field of ["title", "artist", "language"]) {
        document.getElementById(field).value = button.dataset[field] || "";
      }
    }
    if (fresh || !("outExists" in button.dataset)) force.checked = false;
  };

  // A page turn puts the top of the new rows in view. The rows are also drawn again when the
  // editor closes, and that redraw moves nothing.
  let turned = false;

  // Opens one of the two tabs.
  const open = (which) => {
    for (const tab of document.querySelectorAll("[data-tab]")) {
      const on = tab.dataset.tab === which;
      tab.setAttribute("aria-selected", on);
      document.getElementById(tab.getAttribute("aria-controls")).hidden = !on;
    }
  };

  document.body.addEventListener("click", (event) => {
    if (event.target.closest(".pager button")) turned = true;
    const tab = event.target.closest("[data-tab]");
    if (tab) {
      open(tab.dataset.tab);
      return;
    }
    const select = event.target.closest("[data-select]");
    if (select) {
      show(select, true);
      open("words");
      box.focus();
      return;
    }
    if (!event.target.closest("[data-clear]")) return;
    box.value = "";
    use.checked = false;
    held = false;
    mark();
    box.focus();
  });

  // The rows are drawn again when the editor closes and on a page turn. The selected song's row
  // may say something new by then, such as a synced copy that now exists.
  document.body.addEventListener("htmx:afterSwap", () => {
    if (turned) {
      turned = false;
      document.getElementById("rows")?.scrollIntoView({ block: "start" });
    }
    if (!song.value) return;
    const again = [...document.querySelectorAll("[data-select]")].find(
      (button) => button.dataset.select === song.value,
    );
    if (again) show(again, false);
  });

  // A reload keeps what the box and the tick held, where the browser restores them. The selected
  // song is not kept: a song is chosen again on a page that was drawn again.
  song.value = "";
  mark();
})();
