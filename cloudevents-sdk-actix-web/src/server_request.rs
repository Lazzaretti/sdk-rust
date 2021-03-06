use super::headers;
use actix_web::http::HeaderName;
use actix_web::web::{Bytes, BytesMut};
use actix_web::{web, HttpMessage, HttpRequest};
use cloudevents::event::SpecVersion;
use cloudevents::message::{
    BinaryDeserializer, BinarySerializer, Encoding, MessageAttributeValue, MessageDeserializer,
    Result, StructuredDeserializer, StructuredSerializer,
};
use cloudevents::{message, Event};
use futures::StreamExt;
use std::convert::TryFrom;

/// Wrapper for [`HttpRequest`] that implements [`MessageDeserializer`] trait
pub struct HttpRequestDeserializer<'a> {
    req: &'a HttpRequest,
    body: Bytes,
}

impl HttpRequestDeserializer<'_> {
    pub fn new(req: &HttpRequest, body: Bytes) -> HttpRequestDeserializer {
        HttpRequestDeserializer { req, body }
    }
}

impl<'a> BinaryDeserializer for HttpRequestDeserializer<'a> {
    fn deserialize_binary<R: Sized, V: BinarySerializer<R>>(self, mut visitor: V) -> Result<R> {
        if self.encoding() != Encoding::BINARY {
            return Err(message::Error::WrongEncoding {});
        }

        let spec_version = SpecVersion::try_from(
            unwrap_optional_header!(self.req.headers(), headers::SPEC_VERSION_HEADER).unwrap()?,
        )?;

        visitor = visitor.set_spec_version(spec_version.clone())?;

        let attributes = cloudevents::event::spec_version::ATTRIBUTE_NAMES
            .get(&spec_version)
            .unwrap();

        for (hn, hv) in
            self.req.headers().iter().filter(|(hn, _)| {
                headers::SPEC_VERSION_HEADER.ne(hn) && hn.as_str().starts_with("ce-")
            })
        {
            let name = &hn.as_str()["ce-".len()..];

            if attributes.contains(&name) {
                visitor = visitor.set_attribute(
                    name,
                    MessageAttributeValue::String(String::from(header_value_to_str!(hv)?)),
                )?
            } else {
                visitor = visitor.set_extension(
                    name,
                    MessageAttributeValue::String(String::from(header_value_to_str!(hv)?)),
                )?
            }
        }

        if let Some(hv) = self.req.headers().get("content-type") {
            visitor = visitor.set_attribute(
                "datacontenttype",
                MessageAttributeValue::String(String::from(header_value_to_str!(hv)?)),
            )?
        }

        if self.body.len() != 0 {
            visitor.end_with_data(self.body.to_vec())
        } else {
            visitor.end()
        }
    }
}

impl<'a> StructuredDeserializer for HttpRequestDeserializer<'a> {
    fn deserialize_structured<R: Sized, V: StructuredSerializer<R>>(self, visitor: V) -> Result<R> {
        if self.encoding() != Encoding::STRUCTURED {
            return Err(message::Error::WrongEncoding {});
        }
        visitor.set_structured_event(self.body.to_vec())
    }
}

impl<'a> MessageDeserializer for HttpRequestDeserializer<'a> {
    fn encoding(&self) -> Encoding {
        if self.req.content_type() == "application/cloudevents+json" {
            Encoding::STRUCTURED
        } else if self
            .req
            .headers()
            .get::<&'static HeaderName>(&super::headers::SPEC_VERSION_HEADER)
            .is_some()
        {
            Encoding::BINARY
        } else {
            Encoding::UNKNOWN
        }
    }
}

/// Method to transform an incoming [`HttpRequest`] to [`Event`]
pub async fn request_to_event(
    req: &HttpRequest,
    mut payload: web::Payload,
) -> std::result::Result<Event, actix_web::error::Error> {
    let mut bytes = BytesMut::new();
    while let Some(item) = payload.next().await {
        bytes.extend_from_slice(&item?);
    }
    MessageDeserializer::into_event(HttpRequestDeserializer::new(req, bytes.freeze()))
        .map_err(actix_web::error::ErrorBadRequest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::test;
    use url::Url;

    use cloudevents::EventBuilder;
    use serde_json::json;
    use std::str::FromStr;

    #[actix_rt::test]
    async fn test_request() {
        let expected = EventBuilder::new()
            .id("0001")
            .ty("example.test")
            .source(Url::from_str("http://localhost").unwrap())
            .extension("someint", "10")
            .build();

        let (req, payload) = test::TestRequest::post()
            .header("ce-specversion", "1.0")
            .header("ce-id", "0001")
            .header("ce-type", "example.test")
            .header("ce-source", "http://localhost")
            .header("ce-someint", "10")
            .to_http_parts();

        let resp = request_to_event(&req, web::Payload(payload)).await.unwrap();
        assert_eq!(expected, resp);
    }

    #[actix_rt::test]
    async fn test_request_with_full_data() {
        let j = json!({"hello": "world"});

        let expected = EventBuilder::new()
            .id("0001")
            .ty("example.test")
            .source(Url::from_str("http://localhost").unwrap())
            .data("application/json", j.clone())
            .extension("someint", "10")
            .build();

        let (req, payload) = test::TestRequest::post()
            .header("ce-specversion", "1.0")
            .header("ce-id", "0001")
            .header("ce-type", "example.test")
            .header("ce-source", "http://localhost")
            .header("ce-someint", "10")
            .header("content-type", "application/json")
            .set_json(&j)
            .to_http_parts();

        let resp = request_to_event(&req, web::Payload(payload)).await.unwrap();
        assert_eq!(expected, resp);
    }
}
