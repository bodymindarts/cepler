# Concourse integration

Cepler can be integrated into concourse pipelines via the custom resource distributed via the `cepler/cepler-concourse-resource` docker container.
It is important that you name 2 seperate resources of the `get` and `put` operations respectivly.
Due to the way concourse caches and reuses resources things do not work correctly when these aren't seperated.

```
resource_types:
- name: cepler
  type: registry-image
  source:
    repository: cepler/cepler-concourse-resource
    tag: latest

- name: cepler-out
  type: registry-image
  source:
    repository: cepler/cepler-concourse-resource
    tag: latest
```

a simple usage example within a pipeline to deploy a `staging` environment could look like this:
```
- name: deploy-testflight
  serial: true
  plan:
  - in_parallel:
    - { get: pipeline-tasks }
    - { get: cepler-staging, trigger: true }
  - task: deploy-staging
    config:
      platform: linux
      image_resource: (( grab meta.task_image_config ))
      inputs:
      - name: pipeline-tasks
      - name: cepler-staging
        path: repo
      run:
        path: pipeline-tasks/ci/tasks/deploy-staging.sh
  - put: cepler-staging-out
    params:
      repository: cepler-staging
    # environment: staging ## optional environment override

resources:
- name: cepler-staging
  type: cepler
  source:
    uri: (( grab meta.git_uri ))
    branch: (( grab meta.git_branch ))
    private_key: (( grab meta.github_private_key ))
    environment: staging
    config: cepler.yml

- name: cepler-staging-out
  type: cepler-out
  source:
    uri: (( grab meta.git_uri ))
    branch: (( grab meta.git_branch ))
    private_key: (( grab meta.github_private_key ))
    environment: staging
    config: cepler.yml
```

When you get a cepler resource you are provided with the specified repository checkout out to the specified branch with the command `cepler prepare -e <environment> --force-clean` run against it.
Ie only the files you have explicitly specified as belonging to this environment in the `cepler.yml` config file will be present.
All other ones will be deleted.

The `put` operation will commit the state via the command `cepler record -e <environment> --reset-head` and push the changes to the remote repository (after attempting to rebase against the upstream head).

## Source params

| param | required | default | notes |
| --- | --- | --- | --- |
| `uri` | yes | — | git remote URL |
| `branch` | yes | — | branch to deploy from |
| `private_key` | yes | — | ssh key for fetch/push |
| `environment` | no | — | which env in `cepler.yml` this resource tracks. Omit to use the resource only as a `put`-side recorder |
| `config` | no | `cepler.yml` | path to the cepler config |
| `gates_file` | no | — | path to a gates file |
| `gates_branch` | no | — | branch the gates file lives on |
| `ignore_queue` | no | `false` | skip queue ordering |
| `depth` | no | `50` | shallow-clone depth for the `in` step, mirroring the official `git` resource's `depth`. The default of 50 is plenty for most state-tracking repos; cepler deepens on-demand if a history walk reaches the shallow boundary, so correctness is preserved at any value. Set to `0` to opt out and do a full clone. |

## Pipeline generation

Please checkout the (cepler-templates)[https://github.com/bodymindarts/cepler-templates] project to find out more about generating best-practices pipelines.
