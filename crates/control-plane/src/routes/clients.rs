use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

use crate::{
    Client, ClientType, ServerState,
    api::{LookupClientResponse, RegisterClientRequest},
};

use super::common::{error_response, serialize_payload, verify_noise_proof};

#[derive(Serialize)]
struct ClientRegistrationProofPayload<'a> {
    public_key: &'a str,
    owner_id: Option<&'a str>,
    client_type: &'a ClientType,
}

pub(crate) async fn register_client_post(
    State(state): State<ServerState>,
    Json(payload): Json<RegisterClientRequest>,
) -> Response {
    if matches!(payload.client_type, ClientType::Relay) {
        return error_response(StatusCode::BAD_REQUEST, "relay is not allowed in /clients");
    }

    let verified = match verify_noise_proof(
        &state,
        &payload.public_key,
        &payload.noise_handshake,
        |canonical_public_key| {
            serialize_payload(&ClientRegistrationProofPayload {
                public_key: canonical_public_key,
                owner_id: payload.owner_id.as_deref(),
                client_type: &payload.client_type,
            })
        },
    ) {
        Ok(verified) => verified,
        Err(err) => return err.into_response(),
    };

    match state
        .db
        .get_client_by_public_key(&verified.canonical_public_key)
        .await
    {
        Ok(Some(_)) => return error_response(StatusCode::CONFLICT, "client already exists"),
        Ok(None) => {}
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }

    let client = Client::create_client(
        payload.owner_id.as_deref(),
        payload.client_type,
        &verified.canonical_public_key,
    );

    if let Err(err) = state.db.insert_client(&client).await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }

    (StatusCode::CREATED, Json(client)).into_response()
}

pub(crate) async fn lookup_client_get(
    Path(guid): Path<String>,
    State(state): State<ServerState>,
) -> Response {
    let (client, children) = match state.db.get_client_with_children(&guid).await {
        Ok(Some(result)) => result,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "client not found"),
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    (
        StatusCode::OK,
        Json(LookupClientResponse { client, children }),
    )
        .into_response()
}
