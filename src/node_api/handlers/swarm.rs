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

use crate::artifact_service::model::PackageType;
use crate::build_service::event::BuildEventClient;
use crate::docker::error_util::RegistryError;
use crate::network::client::Client;
use crate::node_api::model::cli::{
    RequestDockerBuild, RequestDockerLog, RequestMavenBuild, RequestMavenLog,
};
use crate::transparency_log::log::TransparencyLogService;

use log::debug;
use warp::{http::StatusCode, Rejection, Reply};

pub async fn handle_build_docker(
    request_docker_build: RequestDockerBuild,
    build_event_client: BuildEventClient,
) -> Result<impl Reply, Rejection> {
    let build_id = build_event_client
        .start_build(PackageType::Docker, request_docker_build.image)
        .await
        .map_err(RegistryError::from)?;

    let build_id_as_json = serde_json::to_string(&build_id).map_err(RegistryError::from)?;

    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/json")
        .status(StatusCode::OK)
        .body(build_id_as_json))
}

pub async fn handle_build_maven(
    request_maven_build: RequestMavenBuild,
    build_event_client: BuildEventClient,
) -> Result<impl Reply, Rejection> {
    let build_id = build_event_client
        .start_build(PackageType::Maven2, request_maven_build.gav)
        .await
        .map_err(RegistryError::from)?;

    let build_id_as_json = serde_json::to_string(&build_id).map_err(RegistryError::from)?;

    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/json")
        .status(StatusCode::OK)
        .body(build_id_as_json))
}

pub async fn handle_get_peers(mut p2p_client: Client) -> Result<impl Reply, Rejection> {
    let peers = p2p_client.list_peers().await.map_err(RegistryError::from)?;
    debug!("Got received_peers: {:?}", peers);

    let str_peers: Vec<String> = peers.into_iter().map(|p| p.to_string()).collect();
    let str_peers_as_json = serde_json::to_string(&str_peers).unwrap();

    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/octet-stream")
        .status(StatusCode::OK)
        .body(str_peers_as_json)
        .unwrap())
}

pub async fn handle_get_status(mut p2p_client: Client) -> Result<impl Reply, Rejection> {
    let status = p2p_client.status().await.map_err(RegistryError::from)?;

    let status_as_json = serde_json::to_string(&status).unwrap();

    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/json")
        .status(StatusCode::OK)
        .body(status_as_json)
        .unwrap())
}

pub async fn handle_inspect_log_docker(
    request_docker_log: RequestDockerLog,
    transparency_log_service: TransparencyLogService,
) -> Result<impl Reply, Rejection> {
    let result = transparency_log_service
        .search_transparency_logs(&PackageType::Docker, &request_docker_log.image)
        .map_err(RegistryError::from)?;

    let result_as_json = serde_json::to_string(&result).map_err(RegistryError::from)?;

    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/json")
        .status(StatusCode::OK)
        .body(result_as_json)
        .unwrap())
}

pub async fn handle_inspect_log_maven(
    request_maven_log: RequestMavenLog,
    transparency_log_service: TransparencyLogService,
) -> Result<impl Reply, Rejection> {
    let result = transparency_log_service
        .search_transparency_logs(&PackageType::Maven2, &request_maven_log.gav)
        .map_err(RegistryError::from)?;

    let result_as_json = serde_json::to_string(&result).map_err(RegistryError::from)?;

    Ok(warp::http::response::Builder::new()
        .header("Content-Type", "application/json")
        .status(StatusCode::OK)
        .body(result_as_json)
        .unwrap())
}
