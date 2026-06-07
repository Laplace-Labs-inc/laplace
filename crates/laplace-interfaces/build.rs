// SPDX-License-Identifier: Apache-2.0
use std::io::Result;

fn main() -> Result<()> {
    // 📌 proto 파일들이 있는 디렉토리 경로를 정확히 지정합니다.
    let proto_dir = "proto";

    if let Err(e) = prost_build::compile_protos(
        &[
            format!("{proto_dir}/context.proto"),
            format!("{proto_dir}/error.proto"),
        ],
        &[proto_dir], // proto include 경로
    ) {
        eprintln!("cargo:warning=Proto compilation skipped (protoc not found): {e}");
    }
    Ok(())
}
