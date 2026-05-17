// SPDX-License-Identifier: Apache-2.0
use super::dictionary::StaticDictionary;
use super::error::MeshError;
use serde_json::Value;

/// Fetches OpenAPI/Swagger JSON from a URL and builds a static dictionary of endpoints.
///
/// Extracts all HTTP methods (GET, POST, PUT, DELETE, PATCH, OPTIONS, HEAD) from
/// the `paths` field and creates byte-mapped entries (0x01–0xFE).
pub async fn fetch_and_build_dictionary(url: &str) -> Result<StaticDictionary, MeshError> {
    let response = reqwest::get(url).await?;
    let json: Value = response.json().await?;

    let paths = json
        .get("paths")
        .ok_or_else(|| MeshError::InvalidSchema("Missing 'paths' field".to_string()))?
        .as_object()
        .ok_or_else(|| MeshError::ParseError("'paths' must be an object".to_string()))?;

    let mut dict = StaticDictionary::new();

    for (path, methods) in paths.iter() {
        let methods_obj = methods.as_object().ok_or_else(|| {
            MeshError::ParseError(format!("Invalid path definition for '{}'", path))
        })?;

        for method in methods_obj.keys() {
            let method_upper = method.to_uppercase();
            if ["GET", "POST", "PUT", "DELETE", "PATCH", "OPTIONS", "HEAD"]
                .contains(&method_upper.as_str())
            {
                let endpoint = format!("{} {}", method_upper, path);
                dict.insert(endpoint)?;
            }
        }
    }

    Ok(dict)
}
