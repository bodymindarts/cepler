use super::config::*;
use handlebars::*;
use serde::*;

const BASE_TEMPLATE: &str = include_str!("base.yml");
const BASE_TEMPLATE_NAME: &str = "base";
const RESOURCE_PARTIAL: &str = include_str!("resource.yml");
const RESOURCE_PARTIAL_NAME: &str = "resource";
const JOB_PARTIAL: &str = include_str!("job.yml");
const JOB_PARTIAL_NAME: &str = "job";

pub struct Concourse {
    handlebars: Handlebars<'static>,
    config: Config,
}

impl Concourse {
    pub fn new(config: Config) -> Self {
        let mut handlebars = Handlebars::new();
        handlebars
            .register_partial(RESOURCE_PARTIAL_NAME, RESOURCE_PARTIAL)
            .unwrap();
        handlebars
            .register_partial(JOB_PARTIAL_NAME, JOB_PARTIAL)
            .unwrap();
        handlebars
            .register_template_string(BASE_TEMPLATE_NAME, BASE_TEMPLATE)
            .unwrap();
        Self { handlebars, config }
    }

    pub fn render_pipeline(&self) -> String {
        let data = ConcourseData {
            jobs: self.get_jobs(),
            resources: self.get_resources(),
        };
        self.handlebars.render(BASE_TEMPLATE_NAME, &data).unwrap()
    }

    fn get_jobs(&self) -> Vec<JobData> {
        Vec::new()
    }

    fn get_resources(&self) -> Vec<Resource> {
        let repo = self.repo_conf();
        let mut resources = Vec::new();
        for env in self.environments() {
            if !env.head_filters().is_empty() {
                resources.push(Resource {
                    name: head_resource_name(env),
                    repo_uri: &repo.uri,
                    branch: &repo.branch,
                    paths: env.head_filters(),
                    github_private_key: &repo.private_key,
                });
            }
            if env.propagated_from().is_some() && !env.propagated_filters().is_empty() {
                resources.push(Resource {
                    name: propagated_resource_name(env),
                    repo_uri: &repo.uri,
                    branch: &repo.branch,
                    paths: env.propagated_filters(),
                    github_private_key: &repo.private_key,
                });
            }
        }
        resources
    }

    fn environments(&self) -> impl Iterator<Item = &EnvironmentConfig> {
        self.config.environments.values()
    }
    fn concourse_conf(&self) -> &ConcourseConfig {
        &self.config.concourse.as_ref().unwrap()
    }
    fn repo_conf(&self) -> &RepoConfig {
        &self.concourse_conf().repo
    }
}

fn head_resource_name(env: &EnvironmentConfig) -> String {
    format!("{}-head", env.name)
}

fn propagated_resource_name(env: &EnvironmentConfig) -> String {
    format!("{}-passed-{}", env.name, env.propagated_from().unwrap())
}

#[derive(Debug, Serialize)]
struct ConcourseData<'a> {
    jobs: Vec<JobData>,
    resources: Vec<Resource<'a>>,
}
#[derive(Debug, Serialize)]
struct JobData {
    name: String,
}
#[derive(Debug, Serialize)]
struct Resource<'a> {
    name: String,
    repo_uri: &'a str,
    branch: &'a str,
    github_private_key: &'a str,
    paths: &'a [String],
}
