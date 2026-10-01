// Keeps a song or leaves it out, and a shift-click does the same to every song back to the last box
// clicked.
//
// The anchor is a row index rather than the box itself, because every answer redraws `#songs` and
// the box clicked last is gone by the next click. An anchor on another page is no anchor: the run
// would reach rows the person cannot see.
(() => {
  let anchor = null;

  const keepBox = (target) =>
    target instanceof HTMLInputElement && target.name === "keep" && target.dataset.index;

  // Shift with a mouse button also selects text, and every row in the run would be painted in the
  // selection colour. The box toggles on `click`, so refusing the `mousedown` costs only the focus ring.
  document.body.addEventListener("mousedown", (event) => {
    if (event.shiftKey && keepBox(event.target)) event.preventDefault();
  });

  // `click` rather than `change`, because a `change` event carries no `shiftKey`.
  document.body.addEventListener("click", (event) => {
    const box = event.target;
    if (!keepBox(box)) return;
    const index = Number(box.dataset.index);
    const reachable =
      anchor !== null && document.querySelector(`input[name="keep"][data-index="${anchor}"]`);
    const from = event.shiftKey && reachable ? anchor : index;
    anchor = index;
    const songs = document.getElementById("songs");
    const values = { from, to: index };
    // The box has toggled by the time this runs, and the whole run follows it.
    if (box.checked) values.keep = "on";
    htmx.ajax("POST", `/songs/keep?page=${songs.dataset.page}`, {
      target: "#songs",
      swap: "outerHTML",
      values,
    });
  });
})();

// The folder picker. **Use this folder** puts the path in the box the picker belongs to and closes
// it; nothing is posted, so the box's own form still reads or builds. **Close** only closes.
document.body.addEventListener("click", (event) => {
  const use = event.target.closest("[data-use]");
  const close = event.target.closest("[data-close]");
  const target = use ? use.dataset.use : close ? close.dataset.close : null;
  if (!target) return;
  const box = document.getElementById(target);
  if (use && box) box.value = use.dataset.path;
  const picker = document.getElementById(`browse-${target}`);
  if (picker) picker.innerHTML = "";
  if (box) box.focus();
});
