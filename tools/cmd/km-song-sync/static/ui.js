// The words box and the tick that says to use it. The body carries `has-words` while the tick is
// on, and the stylesheet reads it: a song with no words of its own and no text file beside it can
// start only then.
(() => {
  const box = document.getElementById("words-box");
  const use = document.getElementById("use-words");
  if (!box || !use) return;
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
  // A reload keeps what the box and the tick held, where the browser restores them.
  mark();

  document.body.addEventListener("click", (event) => {
    if (!event.target.closest("[data-clear]")) return;
    box.value = "";
    use.checked = false;
    held = false;
    mark();
    box.focus();
  });
})();
