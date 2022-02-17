/*
   Copyright 2021 JFrog Ltd

   Licensed under the Apache License, Version 2.0 (the "License");
   you may not use this file except in compliance with the License.
   You may obtain a copy of the License at

       http://www.apache.org/licenses/LICENSE-2.0

   Unless required by applicable law or agreed to in writing, software
   distributed under the License is distributed on an "AS IS" BASIS,
   WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
   See the License for the specific language governing permissions and
   limitations under the License.
*/

use super::handlers::*;
use super::HashAlgorithm;
use crate::docker::docker_hub_util::get_docker_hub_auth_token;
use crate::docker::error_util::{RegistryError, RegistryErrorCode};
use crate::network::p2p;
use bytes::{Buf, Bytes};
use futures::stream::{FusedStream, Stream};
use futures::task::{Context, Poll};
use libp2p::PeerId;
use log::{debug, info, trace};
use reqwest::{header};
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{BufReader, Read};
use std::io::prelude::*;
use std::path::PathBuf;
use std::pin::Pin;
use std::result::Result;
use std::str;
use uuid::Uuid;
use warp::{http::StatusCode, Rejection, Reply};

#[derive(Clone, Debug, Default)]
pub struct GetBlobsHandle {
    pending_hash_queries: Vec<String>,
}

impl GetBlobsHandle {
    pub fn new() -> GetBlobsHandle {
        GetBlobsHandle {
            pending_hash_queries: vec![],
        }
    }

    pub fn send(mut self, message: String) {
        self.pending_hash_queries.push(message)
    }
}

impl Stream for GetBlobsHandle {
    type Item = String;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !self.pending_hash_queries.is_empty() {
            return Poll::Ready(self.pending_hash_queries.pop());
        }

        Poll::Pending
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            self.pending_hash_queries.len(),
            Some(self.pending_hash_queries.capacity()),
        )
    }
}

impl FusedStream for GetBlobsHandle {
    fn is_terminated(&self) -> bool {
        false
    }
}

pub async fn handle_get_blobs(
    p2p_client: p2p::Client,
    peer_id: Option<PeerId>,
    name: String,
    hash: String,
) -> Result<impl Reply, Rejection> {
    debug!("Getting blob with hash : {:?}", hash);
    println!("Step 1: do we have the blob locally?");
    let blob_content;

    match get_blob_from_local(&hash).await {
        Ok(blob) => {
            blob_content = blob;
        },
        Err(_) => {
            println!("Step 1: NO, we don't have the blob locally");
            println!("Step 2: Do we have the blob in the Pyrsia network?");

            let blob_stored = get_blob_from_elsewhere(p2p_client, peer_id, &name, &hash).await?;
            if blob_stored {
                println!("Step 2: YES, we have the blob in the Pyrsia network");
                blob_content = get_blob_from_local(&hash).await
                    .map_err(|_| {
                        warp::reject::custom(RegistryError {
                            code: RegistryErrorCode::BlobUnknown,
                        })
                    })?;
                println!("Step 2: We have the blob in the Pyrsia network and now local as well");
            } else {
                println!("Step 2: NO, we don't have the blob in the Pyrsia network");
                return Err(warp::reject::custom(RegistryError {
                    code: RegistryErrorCode::Unknown("PYRSIA_ARTIFACT_STORAGE_ERROR".to_string()),
                }));
            }
        }
    }

/*
    match get_artifact(
        hex::decode(&hash.get(7..).unwrap()).unwrap().as_ref(),
        HashAlgorithm::SHA256,
    ) {
        Ok(content) => {
            println!("Step 1: YES, we have the blob locally");
            info!(
                "Reading blob from local Pyrsia storage: {}",
                &hash.get(7..).unwrap()
            );
            blob_content = content;
        }
        Err(error) => {
            println!("Step 1: NO, we don't have the blob locally");
            info!("Reading blob from elsewhere: {}", hash.get(7..).unwrap());
            debug!(
                "Error while fetching artifact from local Pyrsia, so fetching from elsewhere: {}",
                error.to_string()
            );

            println!("Step 2: Do we have the blob in the Pyrsia network?");
            let blob_stored = get_blob_from_elsewhere(p2p_client, peer_id, &name, &hash).await?;
            if blob_stored {
                println!("Step 2: YES, we have the blob in the Pyrsia network");
                blob_content = get_artifact(
                    hex::decode(&hash.get(7..).unwrap()).unwrap().as_ref(),
                    HashAlgorithm::SHA256,
                )
                .map_err(|_| {
                    warp::reject::custom(RegistryError {
                        code: RegistryErrorCode::BlobUnknown,
                    })
                })?;
                println!("Step 2: We have the blob in the Pyrsia network and now local as well");
            } else {
                println!("Step 2: NO, we don't have the blob in the Pyrsia network");
                return Err(warp::reject::custom(RegistryError {
                    code: RegistryErrorCode::Unknown("PYRSIA_ARTIFACT_STORAGE_ERROR".to_string()),
                }));
            }
        }
    };
*/

    println!("Step out: We have the blob {:?}", hash);
    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/octet-stream")
        .status(StatusCode::OK)
        .body(blob_content)
        .unwrap())
}

pub async fn handle_post_blob(
    name: String,
) -> Result<impl Reply, Rejection> {
    let id = Uuid::new_v4();

    trace!(
        "Getting ready to start new upload for {} - {}",
        name,
        id.to_string()
    );

    create_upload_directory(&name, &id.to_string()).map_err(RegistryError::from)?;

    Ok(warp::http::response::Builder::new()
        .header(
            "Location",
            format!("http://localhost:7878/v2/{}/blobs/uploads/{}", name, id),
        )
        .header("Range", "0-0")
        .status(StatusCode::ACCEPTED)
        .body("")
        .unwrap())
}

pub async fn handle_patch_blob(
    name: String,
    id: String,
    bytes: Bytes,
) -> Result<impl Reply, Rejection> {
    let blob_upload_dest = format!(
        "/tmp/registry/docker/registry/v2/repositories/{}/_uploads/{}/data",
        name, id
    );

    let append = append_to_blob(&blob_upload_dest, bytes).map_err(RegistryError::from)?;

    let range = format!("{}-{}", append.0, append.0 + append.1 - 1);
    debug!("Patch blob range: {}", range);

    Ok(warp::http::response::Builder::new()
        .header(
            "Location",
            format!("http://localhost:7878/v2/{}/blobs/uploads/{}", name, id),
        )
        .header("Range", &range)
        .status(StatusCode::ACCEPTED)
        .body("")
        .unwrap())
}

pub async fn handle_put_blob(
    name: String,
    id: String,
    params: HashMap<String, String>,
    bytes: Bytes,
) -> Result<impl Reply, Rejection> {
    let digest = params.get("digest").ok_or(RegistryError {
        code: RegistryErrorCode::Unknown(String::from("missing digest")),
    })?;

    store_blob_in_filesystem(&name, &id, digest, bytes).map_err(RegistryError::from)?;

    Ok(warp::http::response::Builder::new()
        .header(
            "Location",
            format!("http://localhost:7878/v2/{}/blobs/uploads/{}", name, digest),
        )
        .status(StatusCode::CREATED)
        .body("")
        .unwrap())
}

pub fn append_to_blob(blob: &str, mut bytes: Bytes) -> std::io::Result<(u64, u64)> {
    debug!("Patching blob: {}", blob);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(blob)?;
    let mut total_bytes_read: u64 = 0;
    let initial_file_length: u64;
    initial_file_length = file.metadata()?.len();
    while bytes.has_remaining() {
        let bytes_remaining = bytes.remaining();
        let bytes_to_read = if bytes_remaining <= 4096 {
            bytes_remaining
        } else {
            4096
        };
        total_bytes_read += bytes_to_read as u64;
        let mut b = vec![0; bytes_to_read];
        bytes.copy_to_slice(&mut b);
        file.write_all(&b)?;
    }

    Ok((initial_file_length, total_bytes_read))
}

fn create_upload_directory(name: &str, id: &str) -> std::io::Result<String> {
    let upload_directory = format!(
        "/tmp/registry/docker/registry/v2/repositories/{}/_uploads/{}",
        name, id
    );
    fs::create_dir_all(&upload_directory)?;
    Ok(upload_directory)
}

fn store_blob_in_filesystem(
    name: &str,
    id: &str,
    digest: &str,
    bytes: Bytes,
) -> Result<bool, Box<dyn std::error::Error>> {
    let blob_upload_dest_dir = create_upload_directory(name, &id.to_string())?;
    let mut blob_upload_dest_data = blob_upload_dest_dir.clone();
    blob_upload_dest_data.push_str("/data");
    let append = append_to_blob(&blob_upload_dest_data, bytes)?;

    // check if there is enough local allocated disk space
    let available_space = get_space_available(ARTIFACTS_DIR.as_str());
    if available_space.is_err() {
        return Err(available_space.err().unwrap().to_string().into());
    }
    if append.1 > available_space.unwrap() {
        return Err("Not enough space left to store artifact".into());
    }
    //put blob in artifact manager
    let reader = File::open(blob_upload_dest_data.as_str()).unwrap();

    let push_result = put_artifact(
        hex::decode(&digest.get(7..).unwrap()).unwrap().as_ref(),
        Box::new(reader),
        HashAlgorithm::SHA256,
    )?;

    fs::remove_dir_all(&blob_upload_dest_dir)?;

    Ok(push_result)
}

async fn write_blob_to_local(hash: &str, bytes: Bytes) -> Result<bool, RegistryError> {
    let mut base_path: PathBuf = PathBuf::from(ARTIFACTS_DIR.as_str());
    base_path.push(hash.get(7..).unwrap());
    base_path.set_extension("file");

    append_to_blob(base_path.as_path().to_str().unwrap(), bytes)?;

    Ok(true)
}

async fn get_blob_from_local(hash: &str) -> Result<Vec<u8>, RegistryError> {
    let mut base_path: PathBuf = PathBuf::from(ARTIFACTS_DIR.as_str());
    base_path.push(hash.get(7..).unwrap());
    base_path.set_extension("file");
    let file = File::open(base_path.as_path())?;
    let mut buf_reader: BufReader<File> = BufReader::new(file);
    let mut blob = Vec::new();
    buf_reader.read_to_end(&mut blob).map_err(RegistryError::from)?;
    Ok(blob)
}

async fn get_blob_from_elsewhere(p2p_client: p2p::Client, peer_id: Option<PeerId>, name: &str, hash: &str) -> Result<bool, Rejection> {
    Ok(match peer_id {
        Some(peer) => get_blob_from_other_peer(p2p_client.clone(), peer, name, hash).await,
        None => get_blob_from_docker_hub(name, hash).await?
    })
}

// Request the content of the artifact from other peer
async fn get_blob_from_other_peer(mut p2p_client: p2p::Client, peer_id: PeerId, name: &str, hash: &str) -> bool {
    info!("Reading blob from Pyrsia Node {}: {}", peer_id, hash.get(7..).unwrap());
    let artifact = p2p_client.request_artifact(peer_id, String::from(hash)).await.unwrap();
    match write_blob_to_local(hash, bytes::Bytes::from(artifact)).await {
        Ok(stored) => stored,
        Err(error) => {
            debug!(
                "Error while storing artifact in filesystem: {:?}",
                error
            );
            false
        }
    }
/*
    match p2p_client.request_artifact(peer_id, String::from(hash)).await {
        Ok(artifact) => {
            match write_blob_to_local(hash, bytes::Bytes::from(artifact)).await {
                Ok(stored) => stored,
                Err(error) => {
                    debug!(
                        "Error while storing artifact in filesystem: {:?}",
                        error
                    );
                    false
                }
            }
        },
        Err(error) => {
            debug!(
                "Error while fetching artifact from Pyrsia Node, so fetching from dockerhub: {:?}",
                error
            );
            false
        }
    }*/
}

async fn get_blob_from_docker_hub(name: &str, hash: &str) -> Result<bool, RegistryError> {
    let token = get_docker_hub_auth_token(name).await?;

    get_blob_from_docker_hub_with_token(name, hash, token).await
}

async fn get_blob_from_docker_hub_with_token(
    name: &str,
    hash: &str,
    token: String,
) -> Result<bool, RegistryError> {
    let url = format!(
        "https://registry-1.docker.io/v2/library/{}/blobs/{}",
        name, hash
    );
    debug!("Reading blob from docker.io with url: {}", url);
    let response = reqwest::Client::new()
        .get(url)
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .map_err(RegistryError::from)?;

    debug!("Got blob from docker.io with status {}", response.status());
    let bytes = response.bytes().await.map_err(RegistryError::from)?;

    write_blob_to_local(hash, bytes).await
}
