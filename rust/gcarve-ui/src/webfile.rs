//! Browser file hand-off (web build only): a download of bytes, or a blob: URL for a new tab.

pub fn blob_url(bytes: &[u8], mime: &str) -> Result<String, String> {
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let opts = web_sys::BlobPropertyBag::new();
    opts.set_type(mime);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).map_err(|_| "blob")?;
    Ok(web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| "url")?)
}

pub fn download(name: &str, bytes: &[u8]) -> Result<(), String> {
    use eframe::wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no window")?;
    let document = window.document().ok_or("no document")?;
    let url = blob_url(bytes, "application/octet-stream")?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a").map_err(|_| "a")?.dyn_into().map_err(|_| "a")?;
    a.set_href(&url);
    a.set_download(name);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

