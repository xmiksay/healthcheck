angular.module('healthCheckApp', [])
  .controller('HealthCheckController', ['$http', '$interval', '$timeout', function($http, $interval, $timeout) {
    var healthCheck = this;

    // Configuration
    var API_URL = '/api/services';
    var CONFIG_URL = '/api/config';
    var REFRESH_INTERVAL = 5000; // 5 seconds

    // State
    healthCheck.services = [];
    healthCheck.error = null;
    healthCheck.lastUpdate = null;
    healthCheck.autoRefresh = true;
    healthCheck.showConfigEditor = false;
    healthCheck.configEditorMode = 'visual';
    healthCheck.config = null;
    healthCheck.editConfig = null;
    healthCheck.configJson = '';
    healthCheck.configError = null;
    healthCheck.configSuccess = null;
    healthCheck.showBearerPrompt = false;
    healthCheck.bearerToken = '';
    var refreshTimer = null;

    // Get bearer token from localStorage
    function getBearerToken() {
      return localStorage.getItem('healthcheck_bearer_token') || '';
    }

    // Set bearer token in localStorage
    function setBearerToken(token) {
      if (token) {
        localStorage.setItem('healthcheck_bearer_token', token);
      } else {
        localStorage.removeItem('healthcheck_bearer_token');
      }
    }

    // Get HTTP config with auth header if bearer token exists
    function getHttpConfig() {
      var token = getBearerToken();
      if (token) {
        return {
          headers: {
            'Authorization': 'Bearer ' + token
          }
        };
      }
      return {};
    }

    // Load services from API
    healthCheck.loadServices = function() {
      $http.get(API_URL)
        .then(function(response) {
          healthCheck.services = response.data;
          healthCheck.error = null;
          healthCheck.lastUpdate = new Date();
        })
        .catch(function(error) {
          healthCheck.error = error.statusText || 'Failed to load services';
          console.error('Error loading services:', error);
        });
    };

    // Manual refresh
    healthCheck.refresh = function() {
      healthCheck.loadServices();
    };

    // Load configuration from API
    healthCheck.loadConfig = function() {
      $http.get(CONFIG_URL, getHttpConfig())
        .then(function(response) {
          healthCheck.config = response.data;
          healthCheck.configJson = JSON.stringify(response.data, null, 2);
          healthCheck.editConfig = healthCheck.parseConfigForVisualEditor(response.data);
          healthCheck.configError = null;
          healthCheck.showBearerPrompt = false;
        })
        .catch(function(error) {
          if (error.status === 401) {
            healthCheck.showBearerPrompt = true;
            healthCheck.configError = 'Authentication required. Please enter bearer token.';
          } else {
            healthCheck.configError = 'Failed to load configuration: ' + (error.statusText || error.message);
          }
          console.error('Error loading config:', error);
        });
    };

    // Parse config from API format to visual editor format
    healthCheck.parseConfigForVisualEditor = function(config) {
      var editConfig = {
        telegram_token: config.telegram_token,
        telegram_chat_id: config.telegram_chat_id,
        check_interval_success: config.check_interval_success,
        check_interval_fail: config.check_interval_fail,
        notify_failures: config.notify_failures,
        rereport: config.rereport,
        web_port: config.web_port,
        services: {}
      };

      // Parse services
      for (var uuid in config.services) {
        var service = config.services[uuid];
        var editService = {
          enabled: service.enabled,
          name: service.name,
          description: service.description,
          check_interval_success: service.check_interval_success,
          check_interval_fail: service.check_interval_fail,
          notify_failures: service.notify_failures,
          rereport: service.rereport,
          showAdvanced: false,
          check: {}
        };

        // Determine check type and parse check data
        if (service.check.http) {
          editService.checkType = 'http';
          editService.check.http = {
            url: service.check.http.url,
            expected_status: service.check.http.expected_status
          };
        } else if (service.check.certificate) {
          editService.checkType = 'certificate';
          editService.check.certificate = {
            host: service.check.certificate.host,
            port: service.check.certificate.port,
            days_before_expiry: service.check.certificate.days_before_expiry
          };
        } else if (service.check.tcpPing) {
          editService.checkType = 'tcpPing';
          editService.check.tcpPing = {
            host: service.check.tcpPing.host,
            port: service.check.tcpPing.port,
            timeout_ms: service.check.tcpPing.timeout_ms
          };
        }

        editConfig.services[uuid] = editService;
      }

      return editConfig;
    };

    // Convert visual editor format back to API format
    healthCheck.convertVisualEditorToConfig = function() {
      var config = {
        telegram_token: healthCheck.editConfig.telegram_token,
        telegram_chat_id: healthCheck.editConfig.telegram_chat_id,
        check_interval_success: healthCheck.editConfig.check_interval_success,
        check_interval_fail: healthCheck.editConfig.check_interval_fail,
        notify_failures: healthCheck.editConfig.notify_failures,
        rereport: healthCheck.editConfig.rereport,
        web_port: healthCheck.editConfig.web_port,
        services: {}
      };

      // Convert services
      for (var uuid in healthCheck.editConfig.services) {
        var editService = healthCheck.editConfig.services[uuid];
        var service = {
          enabled: editService.enabled,
          name: editService.name,
          description: editService.description
        };

        // Add optional fields only if set
        if (editService.check_interval_success) {
          service.check_interval_success = editService.check_interval_success;
        }
        if (editService.check_interval_fail) {
          service.check_interval_fail = editService.check_interval_fail;
        }
        if (editService.notify_failures) {
          service.notify_failures = editService.notify_failures;
        }
        if (editService.rereport) {
          service.rereport = editService.rereport;
        }

        // Convert check based on type
        service.check = {};
        if (editService.checkType === 'http') {
          service.check.http = {
            url: editService.check.http.url
          };
          if (editService.check.http.expected_status) {
            service.check.http.expected_status = editService.check.http.expected_status;
          }
        } else if (editService.checkType === 'certificate') {
          service.check.certificate = {
            host: editService.check.certificate.host,
            port: editService.check.certificate.port,
            days_before_expiry: editService.check.certificate.days_before_expiry
          };
        } else if (editService.checkType === 'tcpPing') {
          service.check.tcpPing = {
            host: editService.check.tcpPing.host,
            port: editService.check.tcpPing.port,
            timeout_ms: editService.check.tcpPing.timeout_ms
          };
        }

        config.services[uuid] = service;
      }

      return config;
    };

    // Add new service
    healthCheck.addNewService = function() {
      var newUuid = healthCheck.generateUuid();
      healthCheck.editConfig.services[newUuid] = {
        enabled: true,
        name: '',
        description: '',
        checkType: 'http',
        check: {
          http: {
            url: '',
            expected_status: 200
          }
        },
        showAdvanced: false
      };
    };

    // Remove service
    healthCheck.removeService = function(uuid) {
      delete healthCheck.editConfig.services[uuid];
    };

    // Update check type
    healthCheck.updateCheckType = function(service) {
      service.check = {};
      if (service.checkType === 'http') {
        service.check.http = {
          url: '',
          expected_status: 200
        };
      } else if (service.checkType === 'certificate') {
        service.check.certificate = {
          host: '',
          port: 443,
          days_before_expiry: 30
        };
      } else if (service.checkType === 'tcpPing') {
        service.check.tcpPing = {
          host: '',
          port: 80,
          timeout_ms: 3000
        };
      }
    };

    // Generate UUID v4
    healthCheck.generateUuid = function() {
      return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function(c) {
        var r = Math.random() * 16 | 0;
        var v = c === 'x' ? r : (r & 0x3 | 0x8);
        return v.toString(16);
      });
    };

    // Toggle configuration editor
    healthCheck.toggleConfigEditor = function() {
      healthCheck.showConfigEditor = !healthCheck.showConfigEditor;
      if (healthCheck.showConfigEditor && !healthCheck.config) {
        healthCheck.bearerToken = getBearerToken();
        healthCheck.loadConfig();
      }
      healthCheck.configError = null;
      healthCheck.configSuccess = null;
    };

    // Save configuration
    healthCheck.saveConfig = function() {
      healthCheck.configError = null;
      healthCheck.configSuccess = null;

      var newConfig;

      // Get config based on editor mode
      if (healthCheck.configEditorMode === 'visual') {
        try {
          newConfig = healthCheck.convertVisualEditorToConfig();
        } catch (e) {
          healthCheck.configError = 'Invalid configuration: ' + e.message;
          return;
        }
      } else {
        // Validate JSON
        try {
          newConfig = JSON.parse(healthCheck.configJson);
        } catch (e) {
          healthCheck.configError = 'Invalid JSON: ' + e.message;
          return;
        }
      }

      // Send to API
      $http.put(CONFIG_URL, newConfig, getHttpConfig())
        .then(function() {
          healthCheck.configSuccess = 'Configuration updated successfully! Services are restarting...';
          healthCheck.config = newConfig;
          healthCheck.showBearerPrompt = false;

          // Reload services after a short delay
          $timeout(function() {
            healthCheck.loadServices();
            healthCheck.showConfigEditor = false;
            healthCheck.configSuccess = null;
          }, 2000);
        })
        .catch(function(error) {
          if (error.status === 401) {
            healthCheck.showBearerPrompt = true;
            healthCheck.configError = 'Authentication failed. Please check your bearer token.';
          } else {
            healthCheck.configError = 'Failed to update configuration: ' + (error.data || error.statusText || error.message);
          }
          console.error('Error updating config:', error);
        });
    };

    // Save bearer token
    healthCheck.saveBearerToken = function() {
      setBearerToken(healthCheck.bearerToken);
      healthCheck.showBearerPrompt = false;
      healthCheck.loadConfig();
    };

    // Clear bearer token
    healthCheck.clearBearerToken = function() {
      healthCheck.bearerToken = '';
      setBearerToken('');
      healthCheck.showBearerPrompt = false;
    };

    // Cancel config editing
    healthCheck.cancelConfigEdit = function() {
      healthCheck.showConfigEditor = false;
      healthCheck.configError = null;
      healthCheck.configSuccess = null;
      if (healthCheck.config) {
        healthCheck.configJson = JSON.stringify(healthCheck.config, null, 2);
        healthCheck.editConfig = healthCheck.parseConfigForVisualEditor(healthCheck.config);
      }
    };

    // Count services by state
    healthCheck.countByState = function(stateType) {
      return healthCheck.services.filter(function(service) {
        if (typeof service.state === 'string') {
          return service.state === stateType;
        } else if (typeof service.state === 'object' && service.state !== null) {
          // Handle Rust enum format: { "Failure": "error message" } or "Success"
          if (stateType === 'Failure') {
            return service.state.Failure !== undefined;
          }
          return false;
        }
        return false;
      }).length;
    };

    // Get state class for CSS
    healthCheck.getStateClass = function(state) {
      if (typeof state === 'string') {
        return state.toLowerCase();
      } else if (typeof state === 'object' && state !== null) {
        if (state.Failure !== undefined) {
          return 'failure';
        }
      }
      return 'unknown';
    };

    // Get state label for display
    healthCheck.getStateLabel = function(state) {
      if (typeof state === 'string') {
        return state;
      } else if (typeof state === 'object' && state !== null) {
        if (state.Failure !== undefined) {
          var msg = state.Failure;
          return 'Failure' + (msg ? ': ' + msg : '');
        }
      }
      return 'Unknown';
    };

    // Calculate uptime duration from start time
    healthCheck.getUptime = function(uptimeStart) {
      if (!uptimeStart) {
        return '-';
      }

      var start = new Date(uptimeStart);
      var now = new Date();
      var diffMs = now - start;

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
      } else {
        return seconds + 's';
      }
    };

    // Watch auto-refresh toggle
    healthCheck.$watch = function() {
      return healthCheck.autoRefresh;
    };

    // Setup auto-refresh
    function setupAutoRefresh() {
      if (refreshTimer) {
        $interval.cancel(refreshTimer);
      }

      if (healthCheck.autoRefresh) {
        refreshTimer = $interval(function() {
          healthCheck.loadServices();
        }, REFRESH_INTERVAL);
      }
    }

    // Watch for auto-refresh changes
    var watchAutoRefresh = $interval(function() {
      setupAutoRefresh();
    }, 100);

    $timeout(function() {
      $interval.cancel(watchAutoRefresh);
    }, 500);

    // Initial load
    healthCheck.loadServices();
    setupAutoRefresh();

    // Cleanup on destroy
    healthCheck.$onDestroy = function() {
      if (refreshTimer) {
        $interval.cancel(refreshTimer);
      }
    };
  }]);
