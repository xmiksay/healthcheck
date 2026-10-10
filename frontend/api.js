// HTTP layer for the healthcheck API. Every call resolves with the response
// body, or rejects with { status, message } so the UI can show the server's
// {"error": "..."} text directly.
angular.module('healthCheckApp', [])
  .factory('api', ['$http', function($http) {
    var TOKEN_KEY = 'healthcheck_bearer_token';

    function getToken() {
      return localStorage.getItem(TOKEN_KEY) || '';
    }

    function setToken(token) {
      if (token) {
        localStorage.setItem(TOKEN_KEY, token);
      } else {
        localStorage.removeItem(TOKEN_KEY);
      }
    }

    function toError(response) {
      var message;
      if (response.status === -1) {
        message = 'Server unreachable';
      } else if (response.data && response.data.error) {
        message = response.data.error;
      } else {
        message = 'HTTP ' + response.status + (response.statusText ? ' ' + response.statusText : '');
      }
      throw { status: response.status, message: message };
    }

    function request(method, url, data, admin) {
      var config = { method: method, url: url, data: data, headers: {} };
      var token = getToken();
      if (admin && token) {
        config.headers.Authorization = 'Bearer ' + token;
      }
      return $http(config).then(function(response) {
        return response.data;
      }, toError);
    }

    function admin(method, url, data) {
      return request(method, url, data, true);
    }

    function serviceUrl(id) {
      return '/api/services/' + encodeURIComponent(id);
    }

    return {
      getToken: getToken,
      setToken: setToken,
      status: function() { return request('GET', '/api/status', undefined, false); },
      getSettings: function() { return admin('GET', '/api/settings'); },
      putSettings: function(settings) { return admin('PUT', '/api/settings', settings); },
      listServices: function() { return admin('GET', '/api/services'); },
      createService: function(service) { return admin('POST', '/api/services', service); },
      updateService: function(id, service) { return admin('PUT', serviceUrl(id), service); },
      deleteService: function(id) { return admin('DELETE', serviceUrl(id)); },
      checkService: function(id) { return admin('POST', serviceUrl(id) + '/check'); },
      testCheck: function(check) { return admin('POST', '/api/checks/test', check); }
    };
  }]);
