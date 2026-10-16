use std::time::Instant;

use anyhow::{Context, Result, ensure};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage, MessageDescriptor, MethodDescriptor};
use serde::de::IntoDeserializer;
use serde_json::Value;
use tonic::{
    Status,
    codec::{Codec, DecodeBuf, Decoder, EncodeBuf, Encoder},
    transport::{Channel, ClientTlsConfig, Endpoint},
};

use super::configuration::{CONNECT_TIMEOUT, MAX_WIRE_BYTES};

pub(super) struct Grpc {
    channel: Channel,
    method: MethodDescriptor,
    auth: Option<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>,
}

pub(super) struct Prepared {
    address: Endpoint,
    method: MethodDescriptor,
    auth: Option<tonic::metadata::MetadataValue<tonic::metadata::Ascii>>,
}

impl Grpc {
    pub fn prepare(
        endpoint: &str,
        auth: Option<axum::http::HeaderValue>,
        descriptor: &[u8],
        service: &str,
        method: &str,
    ) -> Result<Prepared> {
        let pool = DescriptorPool::decode(descriptor).context("gRPC 描述文件无效")?;
        let service = pool
            .get_service_by_name(service)
            .context("描述文件中不存在 gRPC 服务")?;
        let method = service
            .methods()
            .find(|entry| entry.name() == method)
            .context("描述文件中不存在 gRPC 方法")?;
        ensure!(
            !method.is_client_streaming() && !method.is_server_streaming(),
            "当前能力契约只接收 unary gRPC 方法"
        );
        let mut address =
            Endpoint::from_shared(endpoint.to_owned())?.connect_timeout(CONNECT_TIMEOUT);
        if endpoint.starts_with("https:") {
            address = address.tls_config(ClientTlsConfig::new().with_webpki_roots())?;
        }
        let auth = auth
            .map(|value| {
                let mut metadata = tonic::metadata::MetadataValue::try_from(value.as_bytes())?;
                metadata.set_sensitive(true);
                Ok::<_, anyhow::Error>(metadata)
            })
            .transpose()?;
        Ok(Prepared {
            address,
            method,
            auth,
        })
    }

    pub async fn invoke(&self, input: &Value, deadline: Instant) -> Result<Value> {
        let input =
            DynamicMessage::deserialize(self.method.input(), input.clone().into_deserializer())
                .context("输入不符合 Protobuf JSON 契约")?;
        ensure!(input.encoded_len() <= MAX_WIRE_BYTES, "gRPC 输入超过配额");
        let mut client = tonic::client::Grpc::new(self.channel.clone())
            .max_encoding_message_size(MAX_WIRE_BYTES)
            .max_decoding_message_size(MAX_WIRE_BYTES);
        client.ready().await.context("远端 gRPC 不可用")?;
        let mut request = tonic::Request::new(input);
        request.set_timeout(
            deadline
                .checked_duration_since(Instant::now())
                .context("gRPC 调用已过期")?,
        );
        if let Some(auth) = &self.auth {
            request.metadata_mut().insert("authorization", auth.clone());
        }
        let path = format!(
            "/{}/{}",
            self.method.parent_service().full_name(),
            self.method.name()
        )
        .parse()?;
        let response = client
            .unary(request, path, DynamicCodec(self.method.output()))
            .await
            .context("远端 gRPC 调用失败")?;
        // 保留普通字段的零值与空值，避免合法的零值结果因省略字段而违反输出契约。
        let options = prost_reflect::SerializeOptions::new().skip_default_fields(false);
        Ok(response
            .into_inner()
            .serialize_with_options(serde_json::value::Serializer, &options)?)
    }
}

impl Prepared {
    pub async fn connect(self) -> Result<Grpc> {
        let channel = self.address.connect().await.context("连接远端 gRPC 失败")?;
        Ok(Grpc {
            channel,
            method: self.method,
            auth: self.auth,
        })
    }
}

// 复用 Prost 的消息编码和 Tonic 的帧、限额及取消，只为动态描述补充 Codec。
struct DynamicCodec(MessageDescriptor);
struct DynamicEncoder;
struct DynamicDecoder(MessageDescriptor);

impl Codec for DynamicCodec {
    type Encode = DynamicMessage;
    type Decode = DynamicMessage;
    type Encoder = DynamicEncoder;
    type Decoder = DynamicDecoder;
    fn encoder(&mut self) -> Self::Encoder {
        DynamicEncoder
    }
    fn decoder(&mut self) -> Self::Decoder {
        DynamicDecoder(self.0.clone())
    }
}

impl Encoder for DynamicEncoder {
    type Item = DynamicMessage;
    type Error = Status;
    fn encode(&mut self, item: Self::Item, dst: &mut EncodeBuf<'_>) -> Result<(), Status> {
        item.encode(dst)
            .map_err(|error| Status::invalid_argument(error.to_string()))
    }
}

impl Decoder for DynamicDecoder {
    type Item = DynamicMessage;
    type Error = Status;
    fn decode(&mut self, src: &mut DecodeBuf<'_>) -> Result<Option<Self::Item>, Status> {
        DynamicMessage::decode(self.0.clone(), src)
            .map(Some)
            .map_err(|error| Status::data_loss(error.to_string()))
    }
}
