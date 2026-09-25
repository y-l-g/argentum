// The media library's upload widget.
//
// The form holds a file input (`[data-media-file]`), a preview region
// (`[data-media-preview]`) and a clear control (`[data-media-clear]`). Picking
// a file draws a preview of what was chosen — an `<img>` for an `image/*` file,
// its name for anything else — and the clear control drops it again.
//
// The clear control is a **reset button**, which is what makes the no-JS
// fallback real: with the script off, the browser resets the form and the file
// input empties. With the script on, the control clears the file input itself
// and cancels that reset — a reset would also drop the owner the user picked,
// and losing a choice nobody asked to lose is not what "clear the file" means.
// So: without the script the whole form resets; with it, only the file and its
// preview go.
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
    const form = control.form;
    const input = form && form.querySelector('[data-media-file]');
    // No file input in reach: leave the control to the browser, which resets
    // the form the markup declared it in.
    if (!input) return;
    // The control is a reset button, so this cancels the reset and empties the
    // file input itself. The rest of the form — the owner picker above all —
    // keeps what the user chose.
    event.preventDefault();
    input.value = '';
    clearPreview(input);
  });
}

if (typeof document !== 'undefined') install();

// Exposed for the Node unit test (`media.test.js`), guarded so the browser
// branch stays inert; see ADR-0014 for the no-build stance.
if (typeof module !== 'undefined' && module.exports) {
  module.exports = { showPreview, clearPreview };
}
