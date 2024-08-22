use std::cmp::min;
use std::{env, thread};
use serenity::async_trait;
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::thread::sleep;
use std::time::{Duration, SystemTime};
use dotenv::dotenv;
use linkify::LinkFinder;
use regex::{Captures, Regex};
use serenity::all::ChannelId;
use serenity::model::gateway::Ready;
use serenity::Client;
use serenity::prelude::*;
use std::sync::{mpsc};
use std::sync::mpsc::Sender;

struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }
}

fn write(mut stream: &TcpStream, string: String) {
    let d = string + "\n";
    let bytes = d.into_bytes();
    stream.flush().unwrap();
    stream.write(&bytes).unwrap();
    stream.flush().unwrap();
}

fn send_initial_commands(stream: &TcpStream) {
    let username = env::var("IRC_USERNAME").unwrap_or("".to_owned());
    let password = env::var("IRC_PASSWORD").unwrap_or("".to_owned());

    write(&stream, format!("PASS {}", password));
    write(&stream, format!("NICK {}", username));
    write(&stream, "JOIN #vietnamese".to_string());
}

fn run_loop(pipe: Sender<String>) -> std::io::Result<()> {
    // setup irc
    let role_regex = Regex::new(r"<@&(\\d{17,19})>").unwrap();
    let finder = LinkFinder::new();
    let action_regex = Regex::new(r"\u0001ACTION( (.+)?)?\u0001").unwrap();

    let mut stream = TcpStream::connect("irc.ppy.sh:6667")?;
    send_initial_commands(&stream);

    let mut reader = BufReader::new(&stream);
    let mut reconnect_if_fail = true;
    let mut reconnect_delay_ms = 0;
    let max_reconnect_delay_ms = 60 * 1000;
    let mut last_ping = SystemTime::now();

    // main loop!
    loop {
        let mut line = String::new();
        let count = reader.read_line(&mut line).unwrap();
        line = line.trim_end().to_string();

        if count > 0 {
            let pieces : Vec<String> = line.splitn(4, ' ')
                .map(|split| split.to_string())
                .collect();

            if pieces[0] == "PING" {
                write(&stream, format!("PONG {}", pieces[1]));
                last_ping = SystemTime::now();
                continue;
            }

            match pieces[1].as_str() {
                "PRIVMSG" => {
                    if !pieces[2].contains("#vietnamese") {
                        continue
                    }

                    let mut msg = pieces[3].to_owned();
                    msg = msg[1..].to_string();
                    msg = msg
                        .replace("@everyone", "at-everyone")
                        .replace("@here", "at-here");
                    msg = role_regex.replace(&*msg, |c: &Captures| {
                        let s = String::from(c[1].to_owned());
                        format!("at-role-{}", s)
                    }).to_string();

                    let m = msg.clone();
                    let links: Vec<_> = finder.links(m.as_str()).collect();

                    for link in links {
                        let s = link.as_str().to_string();
                        msg = msg.replace(s.as_str(), &*format!("<{}>", s));
                    }

                    if let Some(a) = action_regex.captures(&msg) {
                        if let Some(res) = a.get(1) {
                            let result = res.as_str().to_string();
                            msg = format!("(*) {}", result.trim_start());
                        }
                    }

                    let mut author = pieces[0].to_owned();
                    author = author[1..(author.len() - 11)].to_owned();

                    let final_msg = format!("[{}] {}", author, msg);
                    println!("{}", final_msg);

                    if let Err(e) = pipe.send(final_msg) {
                        println!("Error piping message : {:?}", e);
                    }
                }
                "001" => {
                    reconnect_delay_ms = 1000;
                    println!("{}", line); // welcome
                },
                "464" => {
                    eprintln!("Wrong credentials. Please check again.");
                    reconnect_if_fail = false;
                },
                _ => {}
            }
        } else {
            // disconnected!
            if !reconnect_if_fail {
                println!("Refusing to reconnect.");
            } else {
                println!("The other side disconnected.");
                if reconnect_delay_ms != 0 {
                    println!("Delaying reconnection by {} ms.", reconnect_delay_ms);
                    sleep(Duration::from_millis(reconnect_delay_ms));
                }
                reconnect_delay_ms = min(reconnect_delay_ms + 1000, max_reconnect_delay_ms);

                let _ = stream.shutdown(Shutdown::Both);

                let new_stream = TcpStream::connect("irc.ppy.sh:6667")?;
                stream = new_stream;
                reader = BufReader::new(&stream);
                println!("Attempting reconnection.");
                send_initial_commands(&stream);
            }
        }
    }
}

async fn client_loop(mut client: Client) {
    if let Err(why) = client.start().await {
        println!("Discord client error: {why:?}");
    }
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let _ = dotenv();

    let (tx, rx) = mpsc::channel();

    // setup discord
    let intents = GatewayIntents::GUILD_MESSAGES | GatewayIntents::DIRECT_MESSAGES;
    let token = env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN not set");
    let client = Client::builder(&token, intents).event_handler(Handler).await.expect("Error creating client");
    let http = client.http.clone();

    let channels = env::var("TARGET_CHANNELS").unwrap_or(String::new());
    let targets  : Vec<_> = channels
        .split(",")
        .filter(|c| c.to_string().trim().len() > 0)
        .map(|c| {
            return match c.parse::<u64>() {
                Ok(u) => {
                    Some(u)
                }
                Err(_) => {
                    None
                }
            }
        })
        .filter(|c| c.is_some())
        .map(|c| c.unwrap())
        .collect();

    println!("Configured to pipe to the following channels : {:?}", targets);


    thread::spawn(move || {
        run_loop(tx)
    });

    tokio::spawn(client_loop(client));

    for recv in rx {
        for tg in &targets {
            let _ = ChannelId::new(*tg).say(&http, &recv).await;
        }
    }

    Ok(())
}
