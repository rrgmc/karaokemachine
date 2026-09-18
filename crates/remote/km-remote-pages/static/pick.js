// Reads a chosen backup file and posts its text.
//
// **A `FileReader` and an ordinary form field rather than a multipart upload**, and the reason is
// that it collapses two code paths into one. The paste box underneath this input posts the same
// `document` field, so the server has one handler and cannot tell -- and does not need to -- which
// of the two a person used. It also keeps that handler on a `String` body, which is what
// `form.rs` exists for, and adds no `multipart` feature to a workspace table shared with the
// machine and the owner's page.
//
// The `<input type="file">` is really there, so both phone shells still open their native picker:
// Android's `onShowFileChooser` and WebKit's own handling of a file input.
//
// **No prose here.** Everything that could go wrong with the file itself -- not ours, damaged,
// empty, too large -- is decided by the server and worded from its catalog in the viewer's language.
// This script's only failure is a file the browser could not read at all, and it submits an empty
// document for that so the server answers with the same sentence it would for an empty one.
(function () {
  "use strict";

  var input = document.getElementById("pick-file");
  var text = document.getElementById("pick-text");
  var form = document.getElementById("pick-form");
  if (!input || !text || !form) return;

  // Without a `FileReader` the input cannot feed the field, and a control that cannot work is worse
  // than none: hiding it leaves the paste box, which does the whole job.
  if (typeof window.FileReader !== "function") {
    input.hidden = true;
    return;
  }

  input.addEventListener("change", function () {
    var file = input.files && input.files[0];
    if (!file) return; // a cancelled picker; the input can be used again

    var reader = new FileReader();
    reader.onload = function () {
      text.value = String(reader.result || "");
      form.submit();
    };
    reader.onerror = function () {
      // Submit nothing rather than nothing at all: an empty document is a case the server already
      // has a sentence for, and a silent dead end is not.
      text.value = "";
      form.submit();
    };
    reader.readAsText(file);
  });
})();
