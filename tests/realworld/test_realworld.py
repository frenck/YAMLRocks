"""Real-world configuration parsing across ecosystems.

This category points YAMLRocks at large, public configuration repositories from
several ecosystems (Home Assistant, ESPHome, Ansible, Kubernetes, Docker
Compose, and more), referenced as git submodules under the data tree at
``tests/data/realworld/<ecosystem>/<repo>`` (kept apart from the test code here).
It asserts the two guarantees that matter most for adoption:

1. **It parses.** Every YAML file in every repo loads without error in round-trip
   mode (custom tags such as ``!include``/``!secret``/``!vault`` are preserved,
   not resolved, so split configs and absent secret files do not matter here).
2. **It round-trips byte-for-byte.** An unmodified round-trip ``YAMLRocksDocument``
   re-emits its source exactly. This is the project's core promise, exercised
   against thousands of files that humans actually wrote and ship.

Each file is its own parametrized test, with an id of ``ecosystem/repo/path``, so
a failure names the exact file and you can select an ecosystem with, e.g.,
``pytest tests/realworld -k ansible``. The suite **auto-skips** when the
submodules are not checked out, so a plain ``pytest`` without
``git submodule update --init`` still runs green.

A handful of files in these third-party repos are not valid standalone YAML
(a Jinja2 template, a config with a spec-invalid comment). Those are recorded in
``KNOWN_INVALID`` as *strict* xfails: if a future parser change starts accepting
one, the suite fails loudly rather than hiding the change.

Add a repo with
``git submodule add --depth 1 <url> tests/data/realworld/<ecosystem>/<name>``;
see ``README.md`` in this directory.
"""

from __future__ import annotations

import pathlib

import pytest

import yamlrocks

pytestmark = pytest.mark.realworld

REPOS_DIR = pathlib.Path(__file__).resolve().parents[1] / "data" / "realworld"

#: Files that are intentionally not valid standalone YAML, keyed by their
#: ``ecosystem/repo/path`` id, with the reason. Each is asserted to *fail*
#: parsing (strict xfail), so this list cannot silently grow to mask a real
#: regression: an unexpected success fails the suite.
KNOWN_INVALID: dict[str, str] = {
    "home-assistant/thomasloven/lovelace/floorplan.yaml": (
        "Jinja2 template, not standalone YAML (PyYAML rejects it too)"
    ),
    "home-assistant/arsaboo/themes/oxfordblue/oxfordblue.yaml": (
        "comment not preceded by whitespace (\"'#x'# c\"): spec-invalid, "
        "PyYAML happens to be lenient"
    ),
    "home-assistant/jcallaghan/.github/label-commenter-config.yml": (
        "multi-line double-quoted scalar with continuation lines at the block "
        "indent: the spec requires indenting past the block (test-suite case "
        "QB6E is an error), PyYAML is lenient and accepts it"
    ),
    "helm/grafana/charts/enterprise-logs/small.yaml": (
        "multi-line flow mapping whose closing brace sits at the block indent: "
        "the spec requires flow content indented past the block (test-suite case "
        "9C9N is an error), PyYAML is lenient and accepts it"
    ),
    "ansible/dev-sec-hardening/.github/ISSUE_TEMPLATE/bug_report.yml": (
        "multi-line flow sequence (`labels: [`) whose closing `]` sits at the "
        "block indent: the spec requires flow content indented past the block "
        "(test-suite case 9C9N is an error), PyYAML is lenient and accepts it"
    ),
    "ansible/dev-sec-hardening/.github/ISSUE_TEMPLATE/feature_request.yml": (
        "multi-line flow sequence (`labels: [`) whose closing `]` sits at the "
        "block indent: the spec requires flow content indented past the block "
        "(test-suite case 9C9N is an error), PyYAML is lenient and accepts it"
    ),
    # Argo Workflows: a `flags: [` / inline-JSON flow sequence whose closing `]`
    # sits at the block indent. Same 9C9N spec-strict class as above (yamlrocks is
    # correct, PyYAML is lenient).
    "argo-workflows/argo-workflows/examples/exit-handler-slack.yaml": (
        "multi-line flow sequence closing `]` at the block indent (9C9N): the "
        "spec requires flow content indented past the block; PyYAML is lenient"
    ),
    "argo-workflows/argo-workflows/examples/resource-delete-with-flags.yaml": (
        "multi-line flow sequence (`flags: [`) closing `]` at the block indent "
        "(9C9N): the spec requires flow content past the block; PyYAML is lenient"
    ),
    "argo-workflows/argo-workflows/examples/resource-flags.yaml": (
        "multi-line flow sequence (`flags: [`) closing `]` at the block indent "
        "(9C9N): the spec requires flow content past the block; PyYAML is lenient"
    ),
    # Genuinely invalid YAML (PyYAML rejects these too): deliberate negative-test
    # fixtures shipped by the projects.
    "argo-workflows/argo-workflows/test/e2e/smoke/basic-invalid.yaml": (
        "trailing content after a flow collection (`[...]-this-is-invalid-yaml`); "
        "invalid YAML, and named a negative fixture (PyYAML rejects it too)"
    ),
    "opentelemetry/collector/confmap/confmaptest/testdata/invalid.yaml": (
        "unclosed flow collection (`[invalid,`); a negative-test fixture, "
        "invalid YAML (PyYAML rejects it too)"
    ),
    "opentelemetry/collector/confmap/provider/fileprovider/testdata/invalid-yaml.yaml": (
        "unclosed flow collection (`[invalid,`); a negative-test fixture, "
        "invalid YAML (PyYAML rejects it too)"
    ),
    # Template files that render to YAML (like Helm chart templates): the raw
    # source has `{{ ... }}` directives and is not standalone YAML. PyYAML rejects
    # them too. Not auto-skipped by `_is_chart_template` (no sibling Chart.yaml).
    "azure-pipelines/examples/templates/deploy-to-existing-kubernetes-cluster.yml": (
        "Azure Pipelines template with `{{#...}}` macro expressions, not "
        "standalone YAML (PyYAML rejects it too)"
    ),
    "azure-pipelines/examples/templates/resources/k8s/deployment.yml": (
        "Azure Pipelines template with `{{#...}}` macro expressions, not "
        "standalone YAML (PyYAML rejects it too)"
    ),
    "azure-pipelines/examples/templates/resources/k8s/service.yml": (
        "Azure Pipelines template with `{{#...}}` macro expressions, not "
        "standalone YAML (PyYAML rejects it too)"
    ),
    "goss/goss/docs/goss.yaml": (
        "goss template with Go `{{ }}` template directives, not standalone YAML "
        "(PyYAML rejects it too)"
    ),
    "goss/goss/integration-tests/goss/goss-service.yaml": (
        "goss template with Go `{{ }}` template directives, not standalone YAML "
        "(PyYAML rejects it too)"
    ),
    "goss/goss/integration-tests/goss/goss-shared.yaml": (
        "goss template with Go `{{ }}` template directives, not standalone YAML "
        "(PyYAML rejects it too)"
    ),
    "kubernetes/kustomize/functions/examples/fn-framework-application/pkg/exampleapp/v1alpha1/templates/job_worker.template.yaml": (
        "kustomize function Go-template (`{{ .Name }}`), not standalone YAML "
        "(PyYAML rejects it too)"
    ),
    "cloud-init/cloud-init/.github/actionlint.yml": (
        'a stray trailing `"` after a double-quoted scalar (a quoting typo in the '
        "upstream file): invalid YAML, PyYAML rejects it too"
    ),
    "argo-workflows/argo-cd/reposerver/repository/testdata/app-parameters/multi/.argocd-source-broken.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "argo-workflows/argo-cd/reposerver/repository/testdata/invalid-manifests-skipped/bad.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "argo-workflows/argo-cd/reposerver/repository/testdata/invalid-manifests/bad.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "argo-workflows/argo-cd/resource_customizations/db.atlasgo.io/AtlasSchema/testdata/healthy.yaml": (
        "multi-line quoted scalar continued at the block indent (QB6E spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "argo-workflows/argo-cd/resource_customizations/work.karmada.io/ClusterResourceBinding/testdata/progressing.yaml": (
        "multi-line quoted scalar continued at the block indent (QB6E spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "argo-workflows/argo-cd/resource_customizations/work.karmada.io/ResourceBinding/testdata/progressing.yaml": (
        "multi-line quoted scalar continued at the block indent (QB6E spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "gitops/flux2/cmd/flux/testdata/trace/deployment.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "gitops/flux2/cmd/flux/testdata/trace/helmrelease-oci.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "gitops/flux2/cmd/flux/testdata/trace/helmrelease.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "gitops/flux2/cmd/flux/testdata/tree/kustomizations.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "opentelemetry/demo/compose.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "prometheus/kube-prometheus/examples/alertmanager-config-with-template.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/activemq-private-lambda-java-sam/ActiveMQAndClientEC2.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/apigw-custom-domain-edge/template.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/apigw-lambda-rust/template.yml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/apigw-ses-transformation/template.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/appsync-lambda-sfn-sam/template.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/documentdb-lambda-java-sam/DocumentDBAndMongoClientEC2.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/eventbridge-webhooks/2-github/template.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/lambda-iot-sam/template.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/msk-lambda-iam-java-sam/MSKAndKafkaClientEC2.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/msk-lambda-iam-node-sam/MSKAndKafkaClientEC2.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/msk-lambda-iam-python-sam/MSKAndKafkaClientEC2.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/msk-lambda-schema-avro-java-sam/MSKAndKafkaClientEC2.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/rabbitmq-private-lambda-java-sam/RabbitMQAndClientEC2.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/rds-sns-event-notification/template.yaml": (
        "an intentionally-invalid negative-test fixture (PyYAML rejects it too)"
    ),
    "serverless/serverless-patterns/systems-manager-automation-to-stepfunctions/template.yaml": (
        "a Go/`{{ }}`-templated file, not standalone YAML (PyYAML rejects it too)"
    ),
    "tekton/pipeline/config/controller.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/alpha/pipelinerun-large-results.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/beta/propagated-pipeline-object-param.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/pipeline-object-param-and-result.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/pipeline-object-results.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/pipelinerun-array-results-substitution.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/pipelinerun-param-array-indexing.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/pipelineruns/stepaction-params.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/taskruns/array-default.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/taskruns/beta/param_array_indexing.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/taskruns/beta/propagated-object-parameters.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/taskruns/object-param-result.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/examples/v1/taskruns/stepaction-passing-results.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/pipeline/test/testdata/spire/spire-agent.yaml": (
        "multi-line flow collection whose closing bracket sits at the block indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "tekton/triggers/config/interceptors/core-interceptors-deployment.yaml": (
        "multi-line flow sequence (`args: [`) whose closing `]` sits at the block "
        "indent (9C9N spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    "argo-workflows/argo-rollouts/test/e2e/crds/istio.yaml": (
        "multi-line single-quoted scalar continued at the block indent (QB6E spec "
        "error); yamlrocks rejects, PyYAML is lenient"
    ),
    "opentelemetry/collector-contrib/.github/workflows/build-and-test.yml": (
        "multi-line single-quoted flow scalar continued at the block indent (QB6E "
        "spec error); yamlrocks rejects, PyYAML is lenient"
    ),
    # Mis-indented block structure that puts a block collection in mapping-key
    # position, or a bare key with no `:`. Invalid YAML the fast decoder and
    # PyYAML both reject; the round-trip composer now rejects them too (it
    # previously accepted them, producing nonsense complex keys).
    "argo-workflows/argo-rollouts/test/e2e/expectedfailures/analysis-run-failfast.yaml": (
        "mis-indented block sequence reaching mapping-key position; a deliberate "
        "`expectedfailures` negative fixture (PyYAML rejects it too)"
    ),
    "helm/helm-tool/pkg/cmd/testdata/testcharts/chart-bad-requirements/Chart.yaml": (
        "a dependency's `version:`/`repository:` dedented to the `-` column, "
        "putting a block mapping in key position; a deliberately-bad "
        "`chart-bad-requirements` fixture (PyYAML rejects it too)"
    ),
    "home-assistant/arsaboo/automations/automations_manual.yaml": (
        "a Jinja2 template body (`{%- ... -%}`) with bare lines and no `:`, not "
        "standalone YAML (PyYAML rejects it too)"
    ),
    "home-assistant/nagyrobi/esphome/ble-sensors-ethernet.yaml": (
        "an ESPHome `lambda: |-` block scalar broken by a comment at the key's "
        "own indent, leaving the C++ body as bare mapping keys (PyYAML rejects "
        "it too)"
    ),
    "home-assistant/thomasloven/lovelace/functions/presence.yaml": (
        "a Jinja2-templated Lovelace config (`{% if %}`) that dedents a sequence "
        "into mapping-key position, not standalone YAML (PyYAML rejects it too)"
    ),
}

# opentelemetry-collector-contrib ships Go-templated e2e test fixtures: K8s
# manifests with `{{ ... }}` placeholders the e2e harness substitutes before
# applying them. They are not standalone YAML (PyYAML rejects them too) and are
# not under a Helm chart (no sibling Chart.yaml), so they are listed explicitly
# rather than auto-skipped. Each stays an individual strict xfail.
_OTEL_TEMPLATE_FIXTURES: tuple[str, ...] = (
    "opentelemetry/collector-contrib/cmd/opampsupervisor/supervisor/templates/extratelemetryconfig.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/supervisor/templates/owntelemetry.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/testdata/collector/healthcheck_config.tmpl.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/testdata/supervisor/supervisor_basic.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/testdata/supervisor/supervisor_fallback.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/testdata/supervisor/supervisor_healthcheck_port.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/testdata/supervisor/supervisor_persistence.yaml",
    "opentelemetry/collector-contrib/cmd/opampsupervisor/testdata/supervisor/supervisor_report_status.yaml",
    "opentelemetry/collector-contrib/extension/observer/k8sobserver/testdata/e2e/namespaced/collector/configmap.yaml",
    "opentelemetry/collector-contrib/extension/observer/k8sobserver/testdata/e2e/namespaced/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac_heuristic/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac_heuristic/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac_heuristic/telemetrygen/cronjob.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac_heuristic/telemetrygen/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/telemetrygen/cronjob.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/telemetrygen/daemonset.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/telemetrygen/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/telemetrygen/job.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/clusterrbac/telemetrygen/statefulset.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/container_id_association_only/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/container_id_association_only/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/mixrbac/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/mixrbac/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/mixrbac/telemetrygen/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/namespacedrbac/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/namespacedrbac/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/namespaced_rbac_no_pod_ip/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/namespaced_rbac_no_pod_ip/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/namespaced_rbac_no_pod_ip/telemetrygen/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/namespacedrbac/telemetrygen/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/sharedprocessor/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/sharedprocessor/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/k8sattributesprocessor/testdata/e2e/sharedprocessor/telemetrygen/deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/akamai/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/akamai/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/akamai/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/akamai/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/aks/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/aks/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/aks/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/aks/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/alibaba_ecs/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/alibaba_ecs/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/alibaba_ecs/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/alibaba_ecs/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/azure/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/azure/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/azure/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/azure/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/consul/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/consul/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/consul/collector/serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/digitalocean/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/digitalocean/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/digitalocean/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/digitalocean/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/dynatrace/collector/01-enrichment-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/dynatrace/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/dynatrace/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/dynatrace/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ec2/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ec2/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ec2/collector/serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ecs/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ecs/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ecs/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ecs/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/01-eks-api-cert-secret.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/02-eks-api-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/03-eks-api-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/04-eks-api-service.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/05-fake-sa-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/06-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/07-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/eks/collector/09-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/elasticbeanstalk/collector/01-enrichment-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/elasticbeanstalk/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/elasticbeanstalk/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/elasticbeanstalk/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/env/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/env/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/env/collector/serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/gcp/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/gcp/collector/02-metadata-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/gcp/collector/03-metadata-service.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/gcp/collector/10-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/gcp/collector/11-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/gcp/collector/13-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/heroku/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/heroku/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/heroku/collector/serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/hetzner/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/hetzner/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/hetzner/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/hetzner/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_classic/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_classic/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_classic/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_classic/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_vpc/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_vpc/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_vpc/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/ibmcloud_vpc/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8s_api/collector/01-clusterrole.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8s_api/collector/02-clusterrolebinding.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8s_api/collector/03-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8s_api/collector/04-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8s_api/collector/06-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8snode/collector/01-clusterrole.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8snode/collector/02-clusterrolebinding.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8snode/collector/03-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8snode/collector/04-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/k8snode/collector/06-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/kubeadm/collector/01-role.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/kubeadm/collector/02-rolebinding.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/kubeadm/collector/03-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/kubeadm/collector/04-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/kubeadm/collector/06-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/lambda/collector/01-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/lambda/collector/02-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/lambda/collector/04-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/nova/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/nova/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/nova/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/nova/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/openshift/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/openshift/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/openshift/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/openshift/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/oraclecloud/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/oraclecloud/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/oraclecloud/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/oraclecloud/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/scaleway/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/scaleway/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/scaleway/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/scaleway/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/system/collector/configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/system/collector/deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/system/collector/serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/tencent_cvm/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/tencent_cvm/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/tencent_cvm/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/tencent_cvm/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/upcloud/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/upcloud/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/upcloud/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/upcloud/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/vultr/collector/01-metadata-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/vultr/collector/02-configmap.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/vultr/collector/03-serviceaccount.yaml",
    "opentelemetry/collector-contrib/processor/resourcedetectionprocessor/testdata/e2e/vultr/collector/05-deployment.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/cluster-scoped/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/cluster-scoped/collector/deployment.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/entities-test/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/entities-test/collector/deployment.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/namespace-scoped/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/namespace-scoped/collector/deployment.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/namespace-scoped-multiple-namespaces/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/k8sclusterreceiver/testdata/e2e/namespace-scoped-multiple-namespaces/collector/deployment.yaml",
    "opentelemetry/collector-contrib/receiver/k8seventsreceiver/testdata/e2e/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/k8seventsreceiver/testdata/e2e/collector/deployment.yaml",
    "opentelemetry/collector-contrib/receiver/k8sobjectsreceiver/testdata/e2e/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/k8sobjectsreceiver/testdata/e2e/collector/deployment.yaml",
    "opentelemetry/collector-contrib/receiver/kubeletstatsreceiver/testdata/e2e/collector/configmap.yaml",
    "opentelemetry/collector-contrib/receiver/kubeletstatsreceiver/testdata/e2e/collector/daemonset.yaml",
)
KNOWN_INVALID.update(
    dict.fromkeys(
        _OTEL_TEMPLATE_FIXTURES,
        "Go-templated opentelemetry-collector e2e fixture (`{{ }}`), not "
        "standalone YAML (PyYAML rejects it too)",
    )
)


def _is_chart_template(path: pathlib.Path) -> bool:
    """Whether ``path`` is a Helm chart *template*, not a YAML document.

    A file under a ``templates/`` directory whose chart root (the parent of
    ``templates/``) contains a ``Chart.yaml`` is Go text/template
    (``{{- if ... }}``) that Helm/Argo render *before* the result is parsed as
    YAML. Such files use a ``.yaml`` extension but are not standalone YAML
    (PyYAML rejects them too), so they are not part of the "real YAML" corpus.

    Keying on the sibling ``Chart.yaml`` is ecosystem-agnostic: it catches a
    chart anywhere (including a ``helm-chart/`` inside a Kubernetes demo) while
    leaving genuine YAML in a ``templates/`` directory that is *not* a Helm chart
    - e.g. CloudFormation templates, which are valid YAML.
    """
    rel_parts = path.relative_to(REPOS_DIR).parts
    for i, part in enumerate(rel_parts):
        if part == "templates" and i > 0:
            chart_dir = REPOS_DIR.joinpath(*rel_parts[:i])
            if (chart_dir / "Chart.yaml").exists() or (
                chart_dir / "Chart.yml"
            ).exists():
                return True
    return False


def _is_utf8(path: pathlib.Path) -> bool:
    """Whether ``path`` is UTF-8. This corpus tests UTF-8 YAML round-trips;
    YAMLRocks decodes UTF-8 input, so a UTF-16/32-encoded fixture (e.g. argo-cd's
    deliberate ``utf-16.yaml``) is out of scope, not a failure to record."""
    try:
        path.read_text(encoding="utf-8")
        return True
    except (UnicodeDecodeError, OSError):
        return False


def _discover() -> list[pathlib.Path]:
    """Every ``*.yaml``/``*.yml`` file across the checked-out config repos,
    excluding Helm chart templates (see :func:`_is_chart_template`) and any
    non-UTF-8 file (see :func:`_is_utf8`)."""
    if not REPOS_DIR.is_dir():
        return []
    files = set(REPOS_DIR.rglob("*.yaml")) | set(REPOS_DIR.rglob("*.yml"))
    return sorted(f for f in files if not _is_chart_template(f) and _is_utf8(f))


def _rel(path: pathlib.Path) -> str:
    return path.relative_to(REPOS_DIR).as_posix()


_FILES = _discover()


if not _FILES:

    def test_realworld_corpus_present() -> None:
        """Skip the category when the config submodules are not checked out."""
        pytest.skip(
            "real-world config submodules not checked out; run "
            "`git submodule update --init` to enable the real-world suite"
        )

else:

    def _params() -> list:
        out = []
        for path in _FILES:
            rel = _rel(path)
            marks = []
            if rel in KNOWN_INVALID:
                marks.append(
                    pytest.mark.xfail(
                        reason=KNOWN_INVALID[rel],
                        strict=True,
                        raises=yamlrocks.YAMLRocksDecodeError,
                    )
                )
            out.append(pytest.param(path, id=rel, marks=marks))
        return out

    @pytest.mark.parametrize("path", _params())
    def test_parses_and_round_trips(path: pathlib.Path) -> None:
        """A real config file parses and re-emits byte-for-byte unmodified."""
        raw = path.read_bytes()
        doc = yamlrocks.loads(raw, option=yamlrocks.OPT_ROUND_TRIP)
        # OPT_ROUND_TRIP always yields a YAMLRocksDocument; narrow the loads() union.
        assert isinstance(doc, yamlrocks.YAMLRocksDocument)
        assert doc.to_yaml() == raw

    # -- Home Assistant: enter via configuration.yaml and resolve includes -----

    #: HA repos whose include-graph round-trip is a known (strict) xfail because
    #: the graph references a file the author did not commit (a secret or a
    #: generated credentials JSON), so resolution cannot read it. This is
    #: environmental, not a parser issue. (Byte-exact write-back of unmodified
    #: included files is handled by a per-include source cache, so it is no longer
    #: a source of xfails.)
    _UNCOMMITTED = "include graph references a file not committed to the repo"
    INCLUDE_GRAPH_XFAIL: dict[str, str] = {
        "arsaboo": _UNCOMMITTED,
        "dubhad": _UNCOMMITTED,
        "benct": _UNCOMMITTED,
        "hmmbob": _UNCOMMITTED,
        # service_account: !include hass-ga-75cd5ac0dda2.json (a gitignored
        # Google Assistant credential), so the graph cannot be fully resolved.
        "dshokouhi": _UNCOMMITTED,
    }

    def _ha_config_params() -> list:
        ha_dir = REPOS_DIR / "home-assistant"
        if not ha_dir.is_dir():
            return []
        out = []
        for repo in sorted(p for p in ha_dir.iterdir() if p.is_dir()):
            if not (repo / "configuration.yaml").exists():
                continue
            marks = []
            if repo.name in INCLUDE_GRAPH_XFAIL:
                marks.append(
                    pytest.mark.xfail(
                        reason=INCLUDE_GRAPH_XFAIL[repo.name], strict=True
                    )
                )
            out.append(
                pytest.param(repo, id=f"home-assistant/{repo.name}", marks=marks)
            )
        return out

    @pytest.mark.parametrize("repo", _ha_config_params())
    def test_ha_include_graph_round_trips(repo: pathlib.Path) -> None:
        """Resolving a HA config from ``configuration.yaml`` round-trips.

        Loads the whole include graph with ``OPT_INCLUDES``, then checks the
        write-back guarantees: the root re-emits with its ``!include`` directives
        restored (byte-for-byte), and every resolved source file re-emits exactly
        as it is on disk. This exercises the include resolver and the writable-
        include path against a real split configuration. Known gaps are recorded
        in ``INCLUDE_GRAPH_XFAIL``.
        """
        cfg = repo / "configuration.yaml"
        doc = yamlrocks.load(
            str(cfg), option=yamlrocks.OPT_INCLUDES | yamlrocks.OPT_ROUND_TRIP
        )
        # OPT_ROUND_TRIP always yields a YAMLRocksDocument; narrow the load() union.
        assert isinstance(doc, yamlrocks.YAMLRocksDocument)
        assert doc.to_yaml() == cfg.read_bytes()
        for path_str, emitted in yamlrocks.dump_includes_map(doc).items():
            assert emitted == pathlib.Path(path_str).read_bytes(), path_str
