use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolchainPolicy {
    pub request: String,
}

pub fn parse_toolchain_policy(
    build_data: Option<&serde_json::Value>,
) -> Result<ToolchainPolicy, (String, String)> {
    let mut raw: String = "auto".to_owned();
    if let Some(data) = build_data {
        let obj = data.as_object().ok_or((
            "NS9101".to_owned(),
            "build manifest must be an object".to_owned(),
        ))?;
        if let Some(tc) = obj.get("toolchain") {
            let tc_obj = tc.as_object().ok_or((
                "NS9101".to_owned(),
                "toolchain must be an object".to_owned(),
            ))?;
            if let Some(c) = tc_obj.get("c") {
                raw = c
                    .as_str()
                    .ok_or((
                        "NS9101".to_owned(),
                        "toolchain.c must be a string".to_owned(),
                    ))?
                    .to_owned();
            }
        }
    }
    if raw == "undefined" {
        raw = "auto".to_owned();
    }
    let request = raw.trim().to_owned();
    if request.is_empty() {
        return Err((
            "NS9101".to_owned(),
            "toolchain.c must not be empty".to_owned(),
        ));
    }
    if request == "auto" {
        return Ok(ToolchainPolicy { request });
    }
    if request.chars().any(|c| c.is_whitespace()) {
        return Err((
            "NS9101".to_owned(),
            "toolchain.c must not contain whitespace".to_owned(),
        ));
    }
    let parts: Vec<&str> = request.split('@').collect();
    if parts.len() > 2 {
        return Err((
            "NS9101".to_owned(),
            "toolchain.c must not contain multiple '@'".to_owned(),
        ));
    }
    let family = parts[0];
    if !matches!(family, "clang" | "gcc" | "zig" | "cc") {
        return Err((
            "NS9101".to_owned(),
            format!("unknown toolchain family '{}'", family),
        ));
    }
    if parts.len() == 2 && parts[1].is_empty() {
        return Err((
            "NS9101".to_owned(),
            "toolchain.c version must not be empty".to_owned(),
        ));
    }
    Ok(ToolchainPolicy { request })
}
