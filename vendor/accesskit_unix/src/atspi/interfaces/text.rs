// Copyright 2024 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

use accesskit_atspi_common::{PlatformNode, Rect, TextSnapshot};
use atspi::{CoordType, Granularity, ScrollType};
use serde::{Serialize, Serializer};
use std::{collections::HashMap, fmt};
use zbus::{
    Connection, Message, ObjectServer,
    export::async_trait::async_trait,
    fdo, interface,
    message::{Flags, Header},
    names::{InterfaceName, MemberName},
    object_server::{DispatchResult2, Interface, SignalEmitter},
    zvariant::{OwnedValue, Signature, Type, Value},
};

pub(crate) struct TextReply(pub(crate) TextSnapshot);

impl Type for TextReply {
    const SIGNATURE: &'static Signature = &Signature::Str;
}

impl Serialize for TextReply {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

struct TextMethods {
    node: PlatformNode,
}

pub(crate) struct TextInterface(TextMethods);

impl TextInterface {
    pub fn new(node: PlatformNode) -> Self {
        Self(TextMethods { node })
    }
}

impl TextMethods {
    fn map_error(&self) -> impl '_ + FnOnce(accesskit_atspi_common::Error) -> fdo::Error {
        |error| crate::util::map_error_from_node(&self.node, error)
    }
}

#[interface(name = "org.a11y.atspi.Text")]
impl TextMethods {
    #[zbus(property)]
    fn character_count(&self) -> fdo::Result<i32> {
        self.node.character_count().map_err(self.map_error())
    }

    #[zbus(property)]
    fn caret_offset(&self) -> fdo::Result<i32> {
        self.node.caret_offset().map_err(self.map_error())
    }

    fn get_string_at_offset(
        &self,
        offset: i32,
        granularity: Granularity,
    ) -> fdo::Result<(String, i32, i32)> {
        self.node
            .string_at_offset(offset, granularity)
            .map_err(self.map_error())
    }

    fn get_text(&self, start_offset: i32, end_offset: i32) -> fdo::Result<TextReply> {
        self.node
            .text_snapshot(start_offset, end_offset)
            .map(TextReply)
            .map_err(self.map_error())
    }

    fn set_caret_offset(&self, offset: i32) -> fdo::Result<bool> {
        self.node.set_caret_offset(offset).map_err(self.map_error())
    }

    fn get_attribute_value(&self, offset: i32, attribute_name: &str) -> fdo::Result<String> {
        self.node
            .text_attribute_value(offset, attribute_name)
            .map_err(self.map_error())
    }

    fn get_attributes(
        &self,
        offset: i32,
    ) -> fdo::Result<(HashMap<&'static str, String>, i32, i32)> {
        self.node.text_attributes(offset).map_err(self.map_error())
    }

    fn get_default_attributes(&self) -> fdo::Result<HashMap<&'static str, String>> {
        self.node
            .default_text_attributes()
            .map_err(self.map_error())
    }

    fn get_character_extents(&self, offset: i32, coord_type: CoordType) -> fdo::Result<Rect> {
        self.node
            .character_extents(offset, coord_type)
            .map_err(self.map_error())
    }

    fn get_offset_at_point(&self, x: i32, y: i32, coord_type: CoordType) -> fdo::Result<i32> {
        self.node
            .offset_at_point(x, y, coord_type)
            .map_err(self.map_error())
    }

    fn get_n_selections(&self) -> fdo::Result<i32> {
        self.node.n_selections().map_err(self.map_error())
    }

    fn get_selection(&self, selection_num: i32) -> fdo::Result<(i32, i32)> {
        self.node.selection(selection_num).map_err(self.map_error())
    }

    fn add_selection(&self, start_offset: i32, end_offset: i32) -> fdo::Result<bool> {
        self.node
            .add_selection(start_offset, end_offset)
            .map_err(self.map_error())
    }

    fn remove_selection(&self, selection_num: i32) -> fdo::Result<bool> {
        self.node
            .remove_selection(selection_num)
            .map_err(self.map_error())
    }

    fn set_selection(
        &self,
        selection_num: i32,
        start_offset: i32,
        end_offset: i32,
    ) -> fdo::Result<bool> {
        self.node
            .set_selection(selection_num, start_offset, end_offset)
            .map_err(self.map_error())
    }

    fn get_range_extents(
        &self,
        start_offset: i32,
        end_offset: i32,
        coord_type: CoordType,
    ) -> fdo::Result<Rect> {
        self.node
            .range_extents(start_offset, end_offset, coord_type)
            .map_err(self.map_error())
    }

    fn get_attribute_run(
        &self,
        offset: i32,
        include_defaults: bool,
    ) -> fdo::Result<(HashMap<&'static str, String>, i32, i32)> {
        self.node
            .text_attribute_run(offset, include_defaults)
            .map_err(self.map_error())
    }

    fn scroll_substring_to(
        &self,
        start_offset: i32,
        end_offset: i32,
        scroll_type: ScrollType,
    ) -> fdo::Result<bool> {
        self.node
            .scroll_substring_to(start_offset, end_offset, scroll_type)
            .map_err(self.map_error())
    }

    fn scroll_substring_to_point(
        &self,
        start_offset: i32,
        end_offset: i32,
        coord_type: CoordType,
        x: i32,
        y: i32,
    ) -> fdo::Result<bool> {
        self.node
            .scroll_substring_to_point(start_offset, end_offset, coord_type, x, y)
            .map_err(self.map_error())
    }
}

#[async_trait]
impl Interface for TextInterface {
    fn name() -> InterfaceName<'static> {
        TextMethods::name()
    }

    async fn get(
        &self,
        property: &str,
        server: &ObjectServer,
        connection: &Connection,
        header: Option<&Header<'_>>,
        emitter: &SignalEmitter<'_>,
    ) -> Option<fdo::Result<OwnedValue>> {
        self.0
            .get(property, server, connection, header, emitter)
            .await
    }

    async fn get_all(
        &self,
        server: &ObjectServer,
        connection: &Connection,
        header: Option<&Header<'_>>,
        emitter: &SignalEmitter<'_>,
    ) -> fdo::Result<HashMap<String, OwnedValue>> {
        self.0.get_all(server, connection, header, emitter).await
    }

    fn set<'call>(
        &'call self,
        property: &'call str,
        value: &'call Value<'_>,
        server: &'call ObjectServer,
        connection: &'call Connection,
        header: Option<&'call Header<'_>>,
        emitter: &'call SignalEmitter<'_>,
    ) -> DispatchResult2<'call> {
        self.0
            .set(property, value, server, connection, header, emitter)
    }

    async fn set_mut(
        &mut self,
        property: &str,
        value: &Value<'_>,
        server: &ObjectServer,
        connection: &Connection,
        header: Option<&Header<'_>>,
        emitter: &SignalEmitter<'_>,
    ) -> Option<fdo::Result<()>> {
        self.0
            .set_mut(property, value, server, connection, header, emitter)
            .await
    }

    fn call<'call>(
        &'call self,
        server: &'call ObjectServer,
        connection: &'call Connection,
        message: &'call Message,
        name: MemberName<'call>,
    ) -> DispatchResult2<'call> {
        if name.as_str() != "GetText" {
            return self.0.call(server, connection, message, name);
        }
        DispatchResult2::Async(Box::pin(async move {
            let header = message.header();
            if header.primary().flags().contains(Flags::NoReplyExpected) {
                return Ok(());
            }
            let reply = message
                .body()
                .deserialize::<(i32, i32)>()
                .map_err(|error| fdo::Error::InvalidArgs(error.to_string()))
                .and_then(|(start, end)| self.0.get_text(start, end));
            let result = match reply {
                Ok(reply) => {
                    let byte_len = reply.0.fragments().map(str::len).sum::<usize>();
                    if byte_len < 256 * 1024 {
                        connection.reply(&header, &reply).await
                    } else {
                        let fragments: Vec<_> = reply.0.fragments().collect();
                        match connection.reply_string_fragments(&header, &fragments).await {
                            Ok(()) => {
                                if std::env::var_os("GLIMPSE_TRACE").is_some() {
                                    eprintln!("GLIMPSE_ATSPI_STREAM {byte_len}");
                                }
                                Ok(())
                            }
                            Err(zbus::Error::Unsupported) => {
                                connection.reply(&header, &reply).await
                            }
                            Err(error) => Err(error),
                        }
                    }
                }
                Err(error) => connection.reply_dbus_error(&header, error).await,
            };
            result.map_err(|error| match error {
                zbus::Error::FDO(error) => *error,
                error => fdo::Error::Failed(error.to_string()),
            })
        }))
    }

    fn call_mut<'call>(
        &'call mut self,
        server: &'call ObjectServer,
        connection: &'call Connection,
        message: &'call Message,
        name: MemberName<'call>,
    ) -> DispatchResult2<'call> {
        self.0.call_mut(server, connection, message, name)
    }

    fn introspect_to_writer(&self, writer: &mut dyn fmt::Write, level: usize) {
        self.0.introspect_to_writer(writer, level);
    }
}
