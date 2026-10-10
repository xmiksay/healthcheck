angular.module('healthCheckApp')
  .controller('HealthCheckController', ['api', '$q', '$interval', function(api, $q, $interval) {
    var healthCheck = this;

    var REFRESH_INTERVAL = 5000; // 5 seconds
    var ID_PATTERN = /^[a-z0-9][a-z0-9_-]{0,63}$/;
    var SETTINGS_FIELDS = ['check_interval_success', 'check_interval_fail', 'notify_failures', 'rereport', 'telegram_chat_id'];
    // Per-service overrides use the same names as the global settings.
    var OVERRIDE_FIELDS = SETTINGS_FIELDS;

    // Dashboard state
    healthCheck.services = [];
    healthCheck.error = null;
    healthCheck.lastUpdate = null;
    healthCheck.autoRefresh = true;

    // Editor state
    healthCheck.idPattern = ID_PATTERN;
    healthCheck.showConfigEditor = false;
    healthCheck.editorLoaded = false;
    healthCheck.adminDisabled = false;
    healthCheck.settings = null;
    healthCheck.editServices = [];
    healthCheck.configError = null;
    healthCheck.configSuccess = null;
    healthCheck.showBearerPrompt = false;
    healthCheck.bearerToken = '';
    var refreshTimer = null;

    // Optional fields are only sent when set; 0 is a valid value (rereport).
    function isSet(value) {
      return value !== null && value !== undefined && value !== '';
    }

    function copyFields(src, dst, fields) {
      fields.forEach(function(field) {
        if (isSet(src[field])) {
          dst[field] = src[field];
        }
      });
      return dst;
    }

    // Central admin error handling: 401 asks for a token, 403 locks the editor,
    // anything else is surfaced via the caller-provided setter.
    function handleAdminError(error, setMessage) {
      if (error.status === 401) {
        healthCheck.showBearerPrompt = true;
        healthCheck.bearerToken = api.getToken();
        setMessage('Authentication required: ' + error.message);
      } else if (error.status === 403) {
        healthCheck.adminDisabled = true;
        healthCheck.configError = 'Admin API disabled on server: set api_bearer_token in the config file';
      } else {
        setMessage(error.message);
      }
      console.error('API error:', error);
    }

    function setConfigError(message) {
      healthCheck.configError = message;
    }

    // ---- Dashboard ----

    healthCheck.loadServices = function() {
      api.status()
        .then(function(statuses) {
          healthCheck.services = statuses;
          healthCheck.error = null;
          healthCheck.lastUpdate = new Date();
        })
        .catch(function(error) {
          healthCheck.error = error.message;
        });
    };

    healthCheck.refresh = healthCheck.loadServices;

    // ---- Editor: conversion between API shapes and form models ----

    function checkToForm(check) {
      check = check || {};
      var type = Object.keys(check || {})[0] || 'http';
      var form = { checkType: type, check: {} };
      form.check[type] = angular.copy(check[type]) || {};
      return form;
    }

    function checkFromForm(service) {
      var type = service.checkType;
      var src = service.check[type] || {};
      var out = {};
      if (type === 'http') {
        out.http = copyFields(src, { url: src.url }, ['expected_status', 'timeout_ms']);
      } else if (type === 'certificate') {
        out.certificate = copyFields(src, { host: src.host, port: src.port }, ['days_before_expiry', 'timeout_ms']);
      } else if (type === 'tcpPing') {
        out.tcpPing = copyFields(src, { host: src.host, port: src.port }, ['timeout_ms']);
      }
      return out;
    }

    function serviceToForm(entry) {
      var form = checkToForm(entry.check);
      form.id = entry.id;
      form.isNew = false;
      form.enabled = entry.enabled;
      form.name = entry.name;
      form.description = entry.description;
      form.status = entry.status;
      form.showAdvanced = OVERRIDE_FIELDS.some(function(f) { return isSet(entry[f]); });
      OVERRIDE_FIELDS.forEach(function(f) { form[f] = entry[f]; });
      return form;
    }

    function serviceFromForm(form) {
      var service = {
        enabled: !!form.enabled,
        name: form.name || '',
        description: form.description || '',
        check: checkFromForm(form)
      };
      return copyFields(form, service, OVERRIDE_FIELDS);
    }

    // ---- Editor: loading ----

    healthCheck.loadEditor = function() {
      healthCheck.configError = null;
      $q.all([api.getSettings(), api.listServices()])
        .then(function(results) {
          healthCheck.settings = results[0];
          healthCheck.editServices = results[1].map(serviceToForm);
          healthCheck.editorLoaded = true;
          healthCheck.adminDisabled = false;
          healthCheck.showBearerPrompt = false;
        })
        .catch(function(error) {
          handleAdminError(error, setConfigError);
        });
    };

    healthCheck.toggleConfigEditor = function() {
      healthCheck.showConfigEditor = !healthCheck.showConfigEditor;
      healthCheck.configError = null;
      healthCheck.configSuccess = null;
      if (healthCheck.showConfigEditor && !healthCheck.editorLoaded) {
        healthCheck.loadEditor();
      }
    };

    // Discards unsaved edits by reloading from the server.
    healthCheck.reloadEditor = function() {
      healthCheck.configSuccess = null;
      healthCheck.loadEditor();
    };

    // ---- Editor: global settings ----

    healthCheck.saveSettings = function() {
      healthCheck.configError = null;
      healthCheck.configSuccess = null;
      var body = {};
      SETTINGS_FIELDS.forEach(function(f) { body[f] = healthCheck.settings[f]; });
      api.putSettings(body)
        .then(function(saved) {
          healthCheck.settings = saved;
          healthCheck.configSuccess = 'Global settings saved.';
        })
        .catch(function(error) {
          handleAdminError(error, setConfigError);
        });
    };

    // ---- Editor: services ----

    healthCheck.addNewService = function() {
      healthCheck.editServices.unshift({
        id: '',
        isNew: true,
        enabled: true,
        name: '',
        description: '',
        checkType: 'http',
        check: { http: { url: '', expected_status: 200 } },
        showAdvanced: false
      });
    };

    healthCheck.updateCheckType = function(service) {
      service.check = {};
      service.result = null;
      if (service.checkType === 'http') {
        service.check.http = { url: '', expected_status: 200 };
      } else if (service.checkType === 'certificate') {
        service.check.certificate = { host: '', port: 443, days_before_expiry: 30 };
      } else if (service.checkType === 'tcpPing') {
        service.check.tcpPing = { host: '', port: 80, timeout_ms: 3000 };
      }
    };

    function serviceError(service) {
      return function(message) {
        service.error = message;
      };
    }

    function startAction(service) {
      service.error = null;
      service.notice = null;
      service.busy = true;
    }

    function endAction(service) {
      service.busy = false;
    }

    healthCheck.saveService = function(service) {
      startAction(service);
      var body = serviceFromForm(service);
      var request;
      if (service.isNew) {
        if (!ID_PATTERN.test(service.id || '')) {
          service.error = 'id: must match ^[a-z0-9][a-z0-9_-]{0,63}$';
          endAction(service);
          return;
        }
        body.id = service.id;
        request = api.createService(body);
      } else {
        request = api.updateService(service.id, body);
      }
      request
        .then(function(entry) {
          var index = healthCheck.editServices.indexOf(service);
          var saved = serviceToForm(entry);
          saved.showAdvanced = service.showAdvanced;
          saved.notice = 'Saved.';
          healthCheck.editServices[index] = saved;
          healthCheck.loadServices();
        })
        .catch(function(error) {
          handleAdminError(error, serviceError(service));
        })
        .finally(function() {
          endAction(service);
        });
    };

    healthCheck.removeService = function(service) {
      var index = healthCheck.editServices.indexOf(service);
      if (service.isNew) {
        healthCheck.editServices.splice(index, 1);
        return;
      }
      if (!confirm('Delete service "' + service.id + '"? This cannot be undone.')) {
        return;
      }
      startAction(service);
      api.deleteService(service.id)
        .then(function() {
          healthCheck.editServices.splice(healthCheck.editServices.indexOf(service), 1);
          healthCheck.loadServices();
        })
        .catch(function(error) {
          handleAdminError(error, serviceError(service));
        })
        .finally(function() {
          endAction(service);
        });
    };

    function showResult(service, request) {
      startAction(service);
      service.result = null;
      request()
        .then(function(response) {
          service.result = response.state;
        })
        .catch(function(error) {
          handleAdminError(error, serviceError(service));
        })
        .finally(function() {
          endAction(service);
        });
    }

    // Runs the check as currently edited, without saving it.
    healthCheck.testService = function(service) {
      showResult(service, function() {
        return api.testCheck(checkFromForm(service));
      });
    };

    // Runs the saved check once; not recorded in stats.
    healthCheck.checkNow = function(service) {
      showResult(service, function() {
        return api.checkService(service.id);
      });
    };

    // ---- Bearer token ----

    healthCheck.saveBearerToken = function() {
      api.setToken(healthCheck.bearerToken);
      healthCheck.showBearerPrompt = false;
      healthCheck.configError = null;
      // Keep unsaved edits if the editor was already loaded; the user retries the action.
      if (!healthCheck.editorLoaded) {
        healthCheck.loadEditor();
      }
    };

    healthCheck.clearBearerToken = function() {
      healthCheck.bearerToken = '';
      api.setToken('');
    };

    // ---- State rendering ----

    healthCheck.countByState = function(stateType) {
      return healthCheck.services.filter(function(service) {
        return healthCheck.getStateClass(service.state) === stateType.toLowerCase();
      }).length;
    };

    // State is "Unknown" | "Success" | {"Failure": "reason"}
    healthCheck.getStateClass = function(state) {
      if (state === 'Success') {
        return 'success';
      }
      if (state && typeof state === 'object' && state.Failure !== undefined) {
        return 'failure';
      }
      return 'unknown';
    };

    healthCheck.getStateLabel = function(state) {
      if (typeof state === 'string') {
        return state;
      }
      if (state && typeof state === 'object' && state.Failure !== undefined) {
        return 'Failure' + (state.Failure ? ': ' + state.Failure : '');
      }
      return 'Unknown';
    };

    healthCheck.getUptime = function(uptimeStart) {
      if (!uptimeStart) {
        return '-';
      }

      var diffMs = new Date() - new Date(uptimeStart);
      if (diffMs < 0) {
        return '-';
      }

      var seconds = Math.floor(diffMs / 1000);
      var minutes = Math.floor(seconds / 60);
      var hours = Math.floor(minutes / 60);
      var days = Math.floor(hours / 24);

      seconds = seconds % 60;
      minutes = minutes % 60;
      hours = hours % 24;

      if (days > 0) {
        return days + 'd ' + hours + 'h ' + minutes + 'm';
      } else if (hours > 0) {
        return hours + 'h ' + minutes + 'm ' + seconds + 's';
      } else if (minutes > 0) {
        return minutes + 'm ' + seconds + 's';
      }
      return seconds + 's';
    };

    // ---- Auto-refresh ----

    healthCheck.setupAutoRefresh = function() {
      if (refreshTimer) {
        $interval.cancel(refreshTimer);
        refreshTimer = null;
      }
      if (healthCheck.autoRefresh) {
        refreshTimer = $interval(healthCheck.loadServices, REFRESH_INTERVAL);
      }
    };

    healthCheck.$onDestroy = function() {
      if (refreshTimer) {
        $interval.cancel(refreshTimer);
      }
    };

    healthCheck.loadServices();
    healthCheck.setupAutoRefresh();
  }]);
