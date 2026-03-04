#[cfg(test)]
mod tests {
    use e2e::testkit::{ClientResponseExt, TestRouter, TestSubgraphs};
    use hive_router::{ntex, sonic_rs::JsonValueTrait};

    #[ntex::test]
    async fn forward_files() {
        let subgraphs = TestSubgraphs::builder().build().start().await;

        let router = TestRouter::builder()
            .with_subgraphs(&subgraphs)
            .file_config("../plugin_examples/multipart/router.config.yaml")
            .register_plugin::<crate::plugin::MultipartPlugin>()
            .build()
            .start()
            .await;

        let form = reqwest::multipart::Form::new()
            .text("operations", r#"{"query":"mutation ($file: Upload) { upload(file: $file) }","variables":{"file":null}}"#)
            .text("map", r#"{"0":["variables.file"]}"#)
            .part(
                "0",
                reqwest::multipart::Part::bytes("file content".as_bytes().to_vec())
                    .file_name("test.txt")
                    .mime_str("text/plain")
                    .unwrap(),
            );

        let (boundary, form_bytes) = crate::plugin::form_to_boundary_and_bytes(form).await;

        let res = router
            .serv()
            .post(router.graphql_path())
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .send_body(form_bytes)
            .await
            .unwrap();

        let body_json = res.json_body().await;
        let upload_file_path = body_json["data"]["upload"].as_str().unwrap();
        assert!(
            upload_file_path.contains("test.txt"),
            "Response should contain the filename"
        );
        let file_content = std::fs::read(upload_file_path).unwrap();
        assert_eq!(
            file_content, b"file content",
            "File content should match the uploaded content"
        );
        assert_eq!(
            subgraphs.get_requests_log("products").unwrap().len(),
            1,
            "Expected 1 request to products subgraph"
        );
    }
}
