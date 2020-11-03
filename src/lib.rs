use std::collections::BTreeMap;
use std::rc::Rc;

use reqwest::blocking::{Client, RequestBuilder};
use xmlrpc::{Request, Value};

pub struct TracUser {
    pub username: String,
    pub password: String,
}

pub struct TracConfig {
    pub user: Rc<TracUser>,
    pub host: String,
    pub path: String,
}

#[derive(Debug)]
pub struct TracReviewer {
    pub name: String,
    pub aliases: Vec<String>,
    pub email: String
}

#[derive(Debug)]
pub struct TracAction {
    pub name: String,
    pub description: String,
}

impl TracAction {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            description: "".to_string()
        }
    }
}

#[derive(Debug)]
pub struct TracUpdateAttributes {
    pub action: String,
    //_ts: u64 //??
}

impl TracUpdateAttributes {
    fn new(action: TracAction) -> Self {
        Self {
            action: action.name,
            //_ts: Utc::now().timestamp()
        }
    }
}

#[derive(Debug)]
enum TracTicketFieldType {
    DropDown,
    String,
    Integer,
    Text,
    Float,
    Boolean,
}

#[derive(Debug)]
struct TracTicketField {
    name: String,
    field_type: TracTicketFieldType,
    options: Option<Vec<String>>,
    default: Option<String>,
}

#[derive(Debug)]
struct TracTicketFieldSet {
    fields: Vec<TracTicketField>,
}

impl TracTicketFieldSet {
    fn get(trac: &Trac) -> Result<Self, ()> {
        let transport = trac.get_transport();
        let xmlrpc_req = Request::new("ticket.getTicketFields");

        match xmlrpc_req.call(transport) {
            Ok(r) => {
                let mut fields: Vec<TracTicketField> = Vec::new();
                let result = r.as_array().expect("XMLRPC result was not an array.");
                for field_val in result {
                    if let Some(field_meta) = field_val.as_struct() {
                        if let Value::String(field_name) = &field_meta["name"] {
                            if let Value::String(type_label) = &field_meta["type"] {
                                let field_type = match type_label.as_str() {
                                    "text" => TracTicketFieldType::String,
                                    "textarea" => TracTicketFieldType::Text,
                                    "select" | "radio" => TracTicketFieldType::DropDown,
                                    "checkbox" => TracTicketFieldType::Boolean,
                                    _ => continue,
                                };

                                let field_default =
                                    if let Some((k, val)) = field_meta.get_key_value("default") {
                                        match val {
                                            Value::String(ref d) => {
                                                if d != "" {
                                                    Some(d.to_owned())
                                                } else {
                                                    None
                                                }
                                            }
                                            _ => None,
                                        }
                                    } else {
                                        None
                                    };

                                let field_options =
                                    if let Some((k, val)) = field_meta.get_key_value("options") {
                                        match val {
                                            Value::Array(o) => {
                                                let options = o
                                                    .iter()
                                                    .map(|v| {
                                                        v.as_str()
                                                            .expect("value not a string")
                                                            .to_string()
                                                    })
                                                    .collect();
                                                Some(options)
                                            }
                                            _ => None,
                                        }
                                    } else {
                                        None
                                    };

                                fields.push(TracTicketField {
                                    name: field_name.to_owned(),
                                    field_type: field_type,
                                    options: field_options,
                                    default: field_default,
                                });
                            }
                        }
                    }
                }

                Ok(TracTicketFieldSet { fields })
            }
            Err(e) => {
                eprintln!("\nError: {}\n", e);
                Err(())
            }
        }
    }
}

#[derive(Debug)]
pub struct TracTicket {
    pub id: i32,
    pub summary: String,
    pub description: String,
    pub component: String,
    pub owner: String,
    pub reporter: String,
    pub tester: String,
    pub priority: String,
    pub milestone: String,
    pub status: String,
    pub reviewer: String,
    pub resolution: String,
}

fn val_to_string(val: &Value) -> String {
    val.as_str().unwrap().to_string()
}

fn get_val(valmap: &BTreeMap<String, Value>, field: &str) -> String {
    match valmap.get(field) {
        Some(val) => val_to_string(val),
        None => "".to_string(),
    }
}

impl TracTicket {
    fn get(id: i32, trac: &Trac) -> Result<Self, ()> {
        let transport = trac.get_transport();
        let xmlrpc_req = Request::new("ticket.get").arg(id);

        match xmlrpc_req.call(transport) {
            Ok(r) => {
                let fields = r[3].as_struct().unwrap();
                let t = TracTicket {
                    id: r[0].as_i32().unwrap(),
                    summary: get_val(&fields, "summary"),
                    description: get_val(&fields, "description"),
                    component: get_val(&fields, "component"),
                    reporter: get_val(&fields, "reporter"),
                    owner: get_val(&fields, "owner"),
                    reviewer: get_val(&fields, "reviewer"),
                    tester: get_val(&fields, "tester"),
                    priority: get_val(&fields, "priority"),
                    milestone: get_val(&fields, "milestone"),
                    status: get_val(&fields, "status"),
                    resolution: get_val(&fields, "resolution"),
                };
                Ok(t)
            }
            Err(e) => {
                eprintln!("\nError: {}\n", e);
                Err(())
            }
        }
    }

    pub fn actions(&self, trac: &Trac) -> Vec<TracAction> {
        let transport = trac.get_transport();
        let xmlrpc_req = Request::new("ticket.getActions").arg(self.id);

        match xmlrpc_req.call(transport) {
            Ok(r) => {
                let mut actions: Vec<TracAction> = Vec::new();

                if let Value::Array(v) = r {
                    for item in v.iter() {
                        actions.push(TracAction {
                            name: val_to_string(&item[0]),
                            description: val_to_string(&item[1]),
                        })
                    }
                }

                actions
            }
            Err(e) => {
                eprintln!("\nError: {}\n", e);
                vec![]
            }
        }
    }

    pub fn url(id: i32, trac: &Trac) -> String {
        let conf = &trac.config;
        let scheme = "https";

        format!("{}://{}{}ticket/{}", scheme, &conf.host, &conf.path, id)
    }

    fn modify_attributes(&self, attributes: Vec<(String, String)>, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        let transport = trac.get_transport();
        let modify_comment = match comment {
            Some(c) => c,
            None => "".to_string()
        };

        let mut ticket_attributes: BTreeMap<String, Value> = BTreeMap::new();
        for (key, value) in attributes {
            ticket_attributes.insert(key, Value::String(value));
        }
        let xmlrpc_req = Request::new("ticket.update")
            .arg(self.id)
            .arg(modify_comment)
            .arg(Value::Struct(ticket_attributes));

        match xmlrpc_req.call(transport) {
            Ok(r) => {
                Ok(())
            }
            Err(e) => {
                eprintln!("\nError: {}\n", &e);
                Err(())
            }
        }

    }

    fn apply_action(&self, action: TracAction, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        let transport = trac.get_transport();
        self.modify_attributes(vec![("action".to_string(), action.name)], comment, trac)
    }

    pub fn set_reviewer(&self, reviewer: String, trac: &Trac) -> Result<(), ()> {
        let transport = trac.get_transport();
        self.modify_attributes(vec![("reviewer".to_string(), reviewer)], None, trac)
    }

    pub fn request_review(&self, reviewer: String, trac: &Trac) -> Result<(), ()> {
        self.set_reviewer(reviewer.clone(), trac);
        self.apply_action(TracAction::new("peer_review"), Some(format!("Sent to {} for review", reviewer)), trac)
    }

    pub fn review_fail(&self, reason: String, trac: &Trac) -> Result<(),()> {
        self.apply_action(TracAction::new("reject"), Some(reason), trac)
    }

    pub fn review_pass(&self, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        self.apply_action(TracAction::new("pass_peer_review"), comment, trac)
    }

    pub fn release(&self, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        self.apply_action(TracAction::new("leave"), comment, trac)
    }

    pub fn accept(&self, estimate: bool, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        let action_name = if estimate {
            "accept"
        } else {
            "no_estimate_needed"
        };

        self.apply_action(TracAction::new(action_name), comment, trac)
    }

    pub fn reopen(&self, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        self.apply_action(TracAction::new("reopen"), comment, trac)
    }

    pub fn close(&self, comment: Option<String>, trac: &Trac) -> Result<(),()> {
        self.apply_action(TracAction::new("resolve"), comment, trac)
    }

    pub fn fmt_terse(&self) -> String {
        format!(
            "Ticket {}: '{}' | o: {}, r: {}, m: {} | {}",
            self.id, self.summary, self.owner, self.reviewer, self.milestone, self.status
        )
    }

    pub fn fmt_detail(&self) -> String {
        format!(
            "{}\n========================================================\n\n{}",
            self.fmt_terse(),
            self.description
        )
    }
}

pub struct Trac {
    pub config: Rc<TracConfig>,
}

impl Trac {
    pub fn url(&self) -> String {
        let conf = &self.config;
        let user = &conf.user;
        let scheme = "https";

        format!("{}://{}{}", scheme, &conf.host, &conf.path)
    }

    fn get_transport(&self) -> RequestBuilder {
        let conf = &self.config;
        let user = &conf.user;

        let url_base = format!("{}login/xmlrpc", self.url());

        Client::new()
            .post(&url_base)
            .basic_auth(&user.username, Some(&user.password))
    }

    pub fn get_ticket(&self, id: i32) -> Result<TracTicket, ()> {
        TracTicket::get(id, &self)
    }
}
