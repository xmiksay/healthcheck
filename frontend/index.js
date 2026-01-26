angular.module('healthCheckApp', [])
  .controller('HealthCheckController', ['$http', '$interval', '$timeout', function($http, $interval, $timeout) {
    var healthCheck = this;

    // Configuration
    var API_URL = '/api/services';
    var REFRESH_INTERVAL = 5000; // 5 seconds

    // State
    healthCheck.services = [];
    healthCheck.error = null;
    healthCheck.lastUpdate = null;
    healthCheck.autoRefresh = true;
    var refreshTimer = null;

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
