// KaraokeMachine Admin — the small amount of script the page needs.
//
// **Most of it is about one htmx behavior**: htmx does not swap a non-2xx response. A handler that
// answers 500 with a perfectly good explanation puts nothing on the page at all, so a failure is
// indistinguishable from a button that does nothing.
//
// **What that covers is the fragments and only those** — the four `hx-get` endpoints this program
// polls and swaps. A control that navigates answers a redirect carrying its sentence, and the page
// it lands on draws the banner, so the tray below never sees one.

(function () {
  "use strict";

  const tray = () => document.getElementById("toasts");

  /** Shows one message until it is dismissed or times out. */
  function toast(text) {
    const host = tray();
    if (!host) return;
    const box = document.createElement("div");
    box.className = "toast";
    box.setAttribute("role", "status");
    box.textContent = text;
    box.addEventListener("click", () => box.remove());
    host.appendChild(box);
    // Long enough to read a sentence about a license or a digest, which is what these say.
    setTimeout(() => box.remove(), 12000);
  }

  /** What a failed request should say, preferring the server's own words. */
  function reason(detail) {
    const response = detail && detail.xhr;
    if (!response) return "The page could not reach this program.";
    const body = (response.responseText || "").trim();
    // The handlers answer with a sentence, not a stack trace. If one somehow returns HTML, do not
    // paste a document into a toast.
    if (body && body.length < 400 && !body.startsWith("<")) return body;
    if (response.status === 0) return "This program stopped answering.";
    return `Something went wrong (${response.status}).`;
  }

  document.body.addEventListener("htmx:responseError", (event) => {
    toast(reason(event.detail));
  });

  document.body.addEventListener("htmx:sendError", () => {
    toast("This program stopped answering. Is it still running?");
  });

  // **A plain form post, so `hx-confirm` would never fire.** htmx only intercepts elements it has a
  // trigger on, and Remove is an ordinary form so that it still works with no script at all. This is
  // delegated from the body because the lists it appears in are swapped in after load.
  //
  // With the script missing, Remove removes without asking — which is the right way round: the
  // alternative is a button that does nothing, and what is deleted is a file this program can fetch
  // or build again.
  document.body.addEventListener("submit", (event) => {
    const form = event.target;
    if (!form || !form.dataset || !form.dataset.confirm) return;
    if (!window.confirm(form.dataset.confirm)) event.preventDefault();
  });

  // **An upload says it is still going.**
  //
  // A package runs to two gibibytes and the forward to the machine is allowed an hour, so a form
  // post that draws nothing is a page that cannot be told from a program that has stopped. There is
  // no percentage here on purpose: the wait is the second hop, from this program to the machine, and
  // a bar that reached 100% while the browser finished sending and then sat still for ten minutes
  // would be a more confident lie than an honest sweep.
  //
  // Matched on `enctype` rather than an attribute in the markup, so the Songs, Pictures and Sound
  // forms are covered without the shared templates carrying anything for a script that may not be
  // there. With no script at all this is exactly what it was: a form post that waits.
  document.body.addEventListener("submit", (event) => {
    const form = event.target;
    if (!form || form.enctype !== "multipart/form-data") return;
    if (event.defaultPrevented) return;

    const said = tray()?.dataset.sending;
    const button = event.submitter;

    // **Disabled on the next tick, never in this handler.** A submit button disabled while its own
    // submit event is running is dropped from the submission by some browsers, which turns a slow
    // upload into one that never happens.
    setTimeout(() => {
      if (button) {
        button.disabled = true;
        if (said) button.textContent = said;
      }
      if (form.dataset.sending) return;
      form.dataset.sending = "1";
      const bar = document.createElement("div");
      bar.className = "job";
      bar.innerHTML =
        '<div class="progress working"><span style="width:100%"></span></div>';
      if (said) {
        const line = document.createElement("p");
        line.textContent = said;
        bar.appendChild(line);
      }
      form.insertAdjacentElement("afterend", bar);
    }, 0);
  });

  // **The front door's password box follows the row somebody picks.**
  //
  // What this computer has saved is saved for the machine it is pointed at, so the moment another
  // row is picked the box is the only way in and there is no reason to make somebody open it. The
  // tick goes with it: it opens set for the machine in force, and a machine nobody has chosen yet is
  // not one this computer has been asked to remember. `defaultChecked` is the markup's own answer,
  // so the tick comes back if the first row is picked again.
  //
  // The sentence above the box goes at the same time, by a rule in the stylesheet, which is what
  // covers all of this with the script absent: the box is then one press away rather than open, and
  // the page still works.
  //
  // Delegated because the discovered rows arrive after load.
  document.body.addEventListener("change", (event) => {
    const picked = event.target;
    if (!picked || picked.name !== "row" || !picked.form) return;
    const chosen = picked.value === "chosen";
    const box = picked.form.querySelector("details.retype");
    if (box) box.open = !chosen;
    const remember = picked.form.querySelector('input[name="remember"]');
    if (remember) remember.checked = remember.defaultChecked && chosen;
  });

  // Exposed so a handler's fragment can raise one without a round trip of its own.
  window.kmToast = toast;
})();
