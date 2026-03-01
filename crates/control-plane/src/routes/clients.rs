use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use hoshi_protocol::control_plane::{
    ClientEntry, ClientType, LookupClientResponse, RegisterClientRequest,
};
use serde::Serialize;

use crate::{Client, ClientType as DomainClientType, ServerState};

use super::common::{error_response, serialize_payload, verify_noise_proof};

#[derive(Serialize)]
struct ClientRegistrationProofPayload<'a> {
    public_key: &'a str,
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
        Ok(Some(existing)) => {
            let response_client: ClientEntry = (&existing).into();
            return (StatusCode::OK, Json(response_client)).into_response();
        }
        Ok(None) => {}
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }

    let client_type: DomainClientType = payload.client_type.clone().into();
    let client = Client::create_client(client_type, &verified.canonical_public_key);

    if let Err(err) = state.db.insert_client(&client).await {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string());
    }

    let response_client: ClientEntry = (&client).into();
    (StatusCode::CREATED, Json(response_client)).into_response()
}

pub(crate) async fn lookup_client_get(
    Path(guid): Path<String>,
    State(state): State<ServerState>,
) -> Response {
    let client = match state.db.get_client(&guid).await {
        Ok(Some(result)) => result,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "client not found"),
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    };

    (
        StatusCode::OK,
        Json(LookupClientResponse {
            public_key: client.public_key,
        }),
    )
        .into_response()
}
