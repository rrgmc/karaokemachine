// The words box. The body carries `has-words` while the box holds anything, and the stylesheet
// reads it: a song with no words of its own and no text file beside it can start only then.
(() => {
  const box = document.getElementById("words-box");
  if (!box) return;
  const mark = () => document.body.classList.toggle("has-words", box.value.trim() !== "");
  box.addEventListener("input", mark);
  // A reload keeps what the box held, where the browser restores it.
  mark();

  document.body.addEventListener("click", (event) => {
    if (!event.target.closest("[data-clear]")) return;
    box.value = "";
    mark();
    box.focus();
  });
})();
