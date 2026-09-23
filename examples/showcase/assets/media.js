// The media library's upload widget (GH #248).
//
// The form holds a file input (`[data-media-file]`), a preview region
// (`[data-media-preview]`) and a clear control (`[data-media-clear]`). Picking
// a file draws a preview of what was chosen — an `<img>` for an `image/*` file,
// its name for anything else — and the clear control drops it again.
//
// The clear control is a **reset button**, which is what makes the no-JS
// fallback real: the browser empties the file input by resetting the form, and
// this script only removes the preview it drew. With JavaScript off there is no
// preview to leave behind, and the input still clears.
//
// Document-level delegation (like bulk.js and variant.js), so markup swapped in
// later needs no re-installation. The preview is the browser's own object URL,
// so it is revoked when it is replaced or cleared rather than held for the
// document's lifetime.

// The preview region of the form `input` belongs to, when it renders one.
function previewRegion(input) {
  const form = input.form;
  return form ? form.querySelector('[data-media-preview]') : null;
}

// Drop `input`'s preview and revoke the object URL it was showing.
function clearPreview(input) {
  const region = previewRegion(input);
  if (!region) return;
  const url = region.dataset.objectUrl;
  if (url) URL.revokeObjectURL(url);
  delete region.dataset.objectUrl;
  region.replaceChildren();
  region.hidden = true;
}

// Draw the preview of `input`'s current selection: a thumbnail for an
// `image/*` file, the file's name for anything else. Nothing selected empties
// the region.
function showPreview(input) {
  const region = previewRegion(input);
  if (!region) return;
  clearPreview(input);
  const file = input.files && input.files[0];
  if (!file) return;
  if (typeof file.type === 'string' && file.type.startsWith('image/')) {
    const url = URL.createObjectURL(file);
    const image = document.createElement('img');
    image.src = url;
    image.alt = file.name;
    region.appendChild(image);
    region.dataset.objectUrl = url;
  } else {
    // Not an image: there is no thumbnail to draw, so the name is the preview.
    region.appendChild(document.createTextNode(file.name));
  }
  region.hidden = false;
}

function install() {
  document.addEventListener('change', (event) => {
    const input = event.target;
    if (input.matches && input.matches('[data-media-file]')) showPreview(input);
  });
  document.addEventListener('click', (event) => {
    const control = event.target.closest && event.target.closest('[data-media-clear]');
    if (!control) return;
    // The control resets the form itself — the browser empties the file input —
    // so the click must not be cancelled. All the script owns is the preview.
    const form = control.form;
    const input = form && form.querySelector('[data-media-file]');
    if (input) clearPreview(input);
  });
}

if (typeof document !== 'undefined') install();

// Exposed for the Node unit test (`media.test.js`). There is no JS test runner
// in this workspace and this file must stay a plain browser script loaded
// through `asset!`, so it cannot be an ES module. The guard keeps the browser
// branch inert.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { showPreview, clearPreview };
}
