// The whole suite exercises the tokio-driven mbox reader, so it belongs to the
// async build alone; the lean build compiles no test binary here.
#![cfg(feature = "async")]

use oxedyne_fe2o3_net::{
    //dns::Fqdn,
    email::{
        file::{
            MboxEmailIterator,
        },
        msg::{
            EmailHeader,
            EmailMessage,
        },
    },
    //smtp::{
    //    cmd::SmtpCommand,
    //},
};

use oxedyne_fe2o3_core::{
    prelude::*,
    test::test_it,
};
use oxedyne_fe2o3_text::string::Stringer;

use std::{
    path::Path,
    pin::Pin,
};


pub fn test_email(filter: &'static str) -> Outcome<()> {

    res!(test_it(filter, &["Email message 000", "all", "email"], || {

        // Note: Every line ends with an implicit \n character.
        let wire = "Received: from servera.example.org (servera.example.org [192.168.0.1])\r
 by serverb.example.com (ExampleMTA) with ESMTP id 12345678\r
 for <recipient@example.com>; Fri, 11 Jun 2023 09:30:00 -0500 (EST)\r
From: Sender <sender@example.org>\r
To: Recipient <recipient@example.com>\r
Subject: Test message\r
Date: Fri, 11 Jun 2023 09:30:00 -0500\r
Message-ID: <12345.67890@servera.example.org>\r
\r
This is a test message sent from Server A to Server B.\r
\r
Best regards,\r
Sender\r
.\r\n";

        let byts = wire.as_bytes();
        let mut stream = std::io::Cursor::new(byts);
        let rt = res!(tokio::runtime::Runtime::new());

        let result = rt.block_on(async {
            EmailMessage::read(&mut Pin::new(&mut stream)).await
        });

        let email = res!(result);
        debug!("{:?}", email);
        let expected = EmailMessage {
            from:       fmt!("Sender <sender@example.org>"),
            to:         vec![fmt!("Recipient <recipient@example.com>")],
            subject:    fmt!("Test message"),
            body:       fmt!("This is a test message sent from Server A to Server B.\n\nBest regards,\nSender\n"),
            headers:    vec![
                EmailHeader::Received(fmt!("from servera.example.org (servera.example.org [192.168.0.1]) by serverb.example.com (ExampleMTA) with ESMTP id 12345678 for <recipient@example.com>; Fri, 11 Jun 2023 09:30:00 -0500 (EST)")),
                EmailHeader::Date(fmt!("Fri, 11 Jun 2023 09:30:00 -0500")),
                EmailHeader::MessageId(fmt!("<12345.67890@servera.example.org>")),
            ],
        };
        for line in Stringer::new(fmt!("{:?}", email)).to_lines("  ") {
            test!("{}", line);
        }
        req!(expected, email, "(L: expected, R: actual)");
        Ok(())
    }));

    res!(test_it(filter, &["Mbox reader 000", "all", "email", "mbox"], || {

        // A committed, synthetic mbox: two messages, a folded header, a blank
        // body line and an mboxrd-escaped ">From " body line.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests").join("data").join("mbox_fixture.mbox");
        let mbox_iter = res!(MboxEmailIterator::new(path));
        let mut emails = Vec::new();
        for email in mbox_iter {
            emails.push(res!(email));
        }

        let expected = vec![
            EmailMessage {
                from:       fmt!("Alice <alice@example.org>"),
                to:         vec![fmt!("Bob <bob@example.com>")],
                subject:    fmt!("First fixture message"),
                body:       fmt!("Hello Bob,\n\nFrom the fixture, an escaped envelope line.\nAlice\n"),
                headers:    vec![
                    EmailHeader::Received(fmt!("from mx.example.org by mx.example.com with ESMTP")),
                    EmailHeader::MessageId(fmt!("<first.0001@example.org>")),
                ],
            },
            EmailMessage {
                from:       fmt!("Carol <carol@example.net>"),
                to:         vec![fmt!("Bob <bob@example.com>")],
                subject:    fmt!("Second fixture message"),
                body:       fmt!("Second body line one.\n"),
                headers:    vec![
                    EmailHeader::Date(fmt!("Tue, 2 Jan 2024 11:30:00 +0000")),
                ],
            },
        ];
        req!(expected, emails, "(L: expected, R: actual)");
        Ok(())
    }));

    Ok(())
}
