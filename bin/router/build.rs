use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_GRAPHIQL");
    if env::var_os("CARGO_FEATURE_GRAPHIQL").is_some() {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("missing OUT_DIR"));
    let output_file = out_dir.join("laboratory.html");
    let product_logo = manifest_dir.join("static/product_logo.svg");
    let node_modules_dist = out_dir.join("node_modules/@graphql-hive/laboratory/dist");

    fs::copy(
        manifest_dir.join("package.json"),
        out_dir.join("package.json"),
    )
    .expect("Failed to copy package.json");
    fs::copy(
        manifest_dir.join("package-lock.json"),
        out_dir.join("package-lock.json"),
    )
    .expect("Failed to copy package-lock.json");

    println!("cargo:rerun-if-changed={}", product_logo.display());
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("node_modules").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        manifest_dir.join("package-lock.json").display()
    );

    if !node_modules_dist.exists() {
        let status = Command::new("npm")
            .args([
                "install",
                "--include=dev", // NODE_ENV=production will skip dev deps - make sure they're in
            ])
            .current_dir(out_dir)
            .status()
            .expect("Failed to execute npm install");

        if !status.success() {
            panic!("npm install failed");
        }
    }

    let html = build_inline_laboratory_html(&node_modules_dist, &product_logo);

    fs::write(output_file, html).expect("failed to write generated laboratory.html");
}

fn build_inline_laboratory_html(dist_dir: &Path, product_logo: &Path) -> String {
    let js_contents = fs::read_to_string(dist_dir.join("hive-laboratory.umd.js"))
        .expect("failed to read hive-laboratory.umd.js");
    let editor_worker =
        fs::read_to_string(dist_dir.join("monacoeditorwork/editor.worker.bundle.js"))
            .expect("failed to read editor worker");
    let graphql_worker =
        fs::read_to_string(dist_dir.join("monacoeditorwork/graphql.worker.bundle.js"))
            .expect("failed to read graphql worker");
    let json_worker = fs::read_to_string(dist_dir.join("monacoeditorwork/json.worker.bundle.js"))
        .expect("failed to read json worker");
    let typescript_worker =
        fs::read_to_string(dist_dir.join("monacoeditorwork/ts.worker.bundle.js"))
            .expect("failed to read typescript worker");
    let product_logo_data_url = format!(
        "data:image/svg+xml;base64,{}",
        base64_encode(&fs::read(product_logo).expect("failed to read product logo"))
    );

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <title>Hive Router Laboratory</title>
    <link rel="icon" type="image/svg+xml" href="{product_logo_data_url}" />
    <style>
      html,
      body,
      #root {{
        height: 100%;
      }}

      body {{
        margin: 0;
      }}
    </style>
  </head>
  <body id="body" class="no-focus-outline">
    <noscript>You need to enable JavaScript to run this app.</noscript>
    <div id="root"></div>

    <script>
      function prepareBlob(workerContent) {{
        const blob = new Blob([workerContent], {{ type: "application/javascript" }});
        return URL.createObjectURL(blob);
      }}
      const workers = {{
        editorWorkerService: prepareBlob({editor_worker}),
        typescript: prepareBlob({typescript_worker}),
        json: prepareBlob({json_worker}),
        graphql: prepareBlob({graphql_worker}),
      }};
      self["MonacoEnvironment"] = {{
        globalAPI: false,
        getWorkerUrl: function (_moduleId, label) {{
          return workers[label];
        }},
      }};

{js_contents}

      HiveLaboratory.renderLaboratory(window.document.querySelector("#root"));
    </script>
  </body>
</html>
"##,
        product_logo_data_url = product_logo_data_url,
        editor_worker = js_string_literal(&editor_worker),
        typescript_worker = js_string_literal(&typescript_worker),
        json_worker = js_string_literal(&json_worker),
        graphql_worker = js_string_literal(&graphql_worker),
        js_contents = escape_inline_script(&js_contents),
    )
}

fn escape_inline_script(value: &str) -> String {
    value
        .replace("</script", "<\\/script")
        .replace("<!--", "<\\!--")
        .replace("<script", "<\\script")
}

fn js_string_literal(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escape_inline_script(&escaped)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);

    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | chunk[2] as u32;
        output.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        output.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
        output.push(TABLE[(n & 0x3f) as usize] as char);
    }

    let remainder = chunks.remainder();
    if !remainder.is_empty() {
        let first = remainder[0] as u32;
        let second = remainder.get(1).copied().unwrap_or_default() as u32;
        let n = (first << 16) | (second << 8);

        output.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        output.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        if remainder.len() == 2 {
            output.push(TABLE[((n >> 6) & 0x3f) as usize] as char);
            output.push('=');
        } else {
            output.push('=');
            output.push('=');
        }
    }

    output
}
