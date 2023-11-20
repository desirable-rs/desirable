use crate::{DynEndpoint, Endpoint, IntoResponse, Middleware, Next, Request, Result};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
pub struct Router {
  pub prefix: Option<String>,
  pub middlewares: Vec<Arc<dyn Middleware>>,
  pub routes: HashMap<hyper::Method, route_recognizer::Router<Box<DynEndpoint>>>,
  pub not_found_handler: Box<DynEndpoint>,
}

async fn default_handler(_req: Request) -> impl IntoResponse {
  "handle not found"
}

impl Router {
  pub fn new() -> Self {
    Router {
      prefix: None,
      middlewares: Vec::new(),
      routes: HashMap::new(),
      not_found_handler: Box::new(default_handler),
    }
  }
  pub fn at(&mut self, method: hyper::Method, route: &str, dest: impl Endpoint) {
    let mut path = String::from("");
    if let Some(prefix) = &self.prefix {
      path.push_str(prefix.as_str());
    }
    path.push_str(route);
    self
      .routes
      .entry(method)
      .or_insert_with(route_recognizer::Router::new)
      .add(path.as_str(), Box::new(dest));
  }

  pub fn get(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::GET, route, dest);
  }

  pub fn post(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::POST, route, dest);
  }

  pub fn delete(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::DELETE, route, dest);
  }

  pub fn patch(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::PATCH, route, dest);
  }

  pub fn put(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::PUT, route, dest);
  }

  pub fn options(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::OPTIONS, route, dest);
  }

  pub fn head(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::HEAD, route, dest);
  }
  pub fn trace(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::TRACE, route, dest);
  }
  pub fn connect(&mut self, route: &str, dest: impl Endpoint) {
    self.at(hyper::Method::CONNECT, route, dest);
  }
  pub fn with(&mut self, middleware: impl Middleware) {
    self.middlewares.push(Arc::new(middleware));
  }
  pub fn merge(&mut self, target: Router) {
    self.routes.extend(target.routes);
  }
  pub async fn dispatch(&self, mut req: Request, remote_addr: Arc<SocketAddr>) -> Result {
    let method = req.method();
    let path = req.uri().path();

    let mut params = route_recognizer::Params::new();
    let endpoint = match self.routes.get(method) {
      Some(route) => match route.recognize(path) {
        Ok(m) => {
          params = m.params().to_owned();
          &***m.handler()
        }
        Err(_e) => &*self.not_found_handler,
      },
      None => &*self.not_found_handler,
    };

    req.params = params;
    req.remote_addr = Some(remote_addr);
    let next = Next {
      endpoint: endpoint,
      middlewares: &self.middlewares,
    };
    next.run(req).await
  }
}
